use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use palpo_core::Seqnum;

use crate::core::client::filter::UrlFilter;
use crate::core::client::search::{
    Criteria, EventContext, EventContextResult, GroupingKey, Groupings, OrderBy,
    OwnedRoomIdOrUserId, ResultGroup, ResultRoomEvents, SearchKeys, SearchResult, UserProfile,
};
use crate::core::events::room::member::RoomMemberEventContent;
use crate::core::events::{StateEventType, TimelineEventType};
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonObject;
use crate::core::serde::canonical_json::CanonicalJsonValue;
use crate::data::connect;
use crate::data::full_text_search::*;
use crate::data::schema::*;
use crate::event::BatchToken;
use crate::room::{state, timeline};
use crate::{AppResult, MatrixError, SnPduEvent, room};

/// The event types the search index covers, with the `key` their text is stored under.
///
/// `content.body` of `m.room.message` events is stored as `content.message`, which is what
/// existing index rows use.
const INDEXED_EVENT_TYPES: [(&str, &str); 3] = [
    ("m.room.message", "content.message"),
    ("m.room.name", "content.name"),
    ("m.room.topic", "content.topic"),
];

/// The largest number of context events returned on either side of a result.
const MAX_CONTEXT_LIMIT: u64 = 100;

/// A search hit in result order: the event, its room and its sender.
type Hit = (OwnedEventId, OwnedRoomId, OwnedUserId);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SearchCursor {
    rank: Option<f32>,
    timestamp: i64,
    event_sn: i64,
    room_id: Option<OwnedRoomId>,
    sender: Option<OwnedUserId>,
}

impl SearchCursor {
    fn parse(token: &str, ranked: bool) -> Result<Self, MatrixError> {
        let invalid = || MatrixError::invalid_param("Invalid search pagination token.");
        let cursor: Self = if let Some(encoded) = token.strip_prefix("s1.") {
            if encoded.len() > 2048 {
                return Err(invalid());
            }
            let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
            serde_json::from_slice(&bytes).map_err(|_| invalid())?
        } else if !ranked {
            // Continue accepting recent-order tokens issued before scoped pagination.
            let (timestamp, event_sn) = token.rsplit_once('-').ok_or_else(invalid)?;
            Self {
                rank: None,
                timestamp: timestamp.parse().map_err(|_| invalid())?,
                event_sn: event_sn.parse().map_err(|_| invalid())?,
                room_id: None,
                sender: None,
            }
        } else {
            return Err(invalid());
        };
        if cursor.timestamp < 0
            || cursor.event_sn < 0
            || cursor.rank.is_some() != ranked
            || cursor
                .rank
                .is_some_and(|rank| !rank.is_finite() || rank < 0.0)
        {
            return Err(invalid());
        }
        Ok(cursor)
    }

    fn encode(&self) -> String {
        format!(
            "s1.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("finite cursor"))
        )
    }
}

fn ranked_search(criteria: &Criteria) -> bool {
    criteria
        .order_by
        .as_ref()
        .is_none_or(|order| *order != OrderBy::Recent)
}

/// The index keys a search has to look at, from the requested `keys` and the filter's
/// `types` and `not_types`. Empty when the criteria exclude every indexed event type.
fn searched_keys(criteria: &Criteria) -> Vec<&'static str> {
    let filter = &criteria.filter;
    INDEXED_EVENT_TYPES
        .iter()
        .filter(|(event_type, key)| {
            if let Some(keys) = &criteria.keys
                && !keys.iter().any(|k| index_key(k) == Some(key))
            {
                return false;
            }
            if let Some(types) = &filter.types
                && !types.iter().any(|p| event_type_matches(p, event_type))
            {
                return false;
            }
            !filter
                .not_types
                .iter()
                .any(|p| event_type_matches(p, event_type))
        })
        .map(|(_, key)| *key)
        .collect()
}

fn index_key(key: &SearchKeys) -> Option<&'static str> {
    match key {
        SearchKeys::ContentBody => Some("content.message"),
        SearchKeys::ContentName => Some("content.name"),
        SearchKeys::ContentTopic => Some("content.topic"),
        _ => None,
    }
}

/// Matches an event type against a filter pattern, in which `*` matches any sequence of
/// characters.
fn event_type_matches(pattern: &str, event_type: &str) -> bool {
    let mut segments = pattern.split('*');
    let prefix = segments.next().unwrap_or_default();
    let Some(mut rest) = event_type.strip_prefix(prefix) else {
        return false;
    };
    let mut segments: Vec<&str> = segments.collect();
    let Some(suffix) = segments.pop() else {
        // No wildcard: the whole type must match.
        return rest.is_empty();
    };
    for segment in segments {
        match rest.find(segment) {
            Some(index) => rest = &rest[index + segment.len()..],
            None => return false,
        }
    }
    rest.ends_with(suffix)
}

fn searchable_events<'a>(
    room_ids: &'a [OwnedRoomId],
    criteria: &'a Criteria,
    keys: &'a [&'static str],
) -> event_searches::BoxedQuery<'a, diesel::pg::Pg> {
    let filter = &criteria.filter;
    let mut visible_events = events::table
        .filter(events::is_redacted.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(events::is_outlier.eq(false))
        .filter(events::soft_failed.eq(false))
        .into_boxed();
    if let Some(url_filter) = &filter.url_filter {
        visible_events = visible_events
            .filter(events::contains_url.eq(matches!(url_filter, UrlFilter::EventsWithUrl)));
    }
    let mut query = event_searches::table
        .filter(event_searches::room_id.eq_any(room_ids))
        .filter(event_searches::key.eq_any(keys))
        .filter(event_searches::event_id.eq_any(visible_events.select(events::id)))
        .filter(event_searches::vector.matches(websearch_to_tsquery(&criteria.search_term)))
        .into_boxed();
    if let Some(senders) = &filter.senders {
        query = query.filter(event_searches::sender_id.eq_any(senders));
    }
    if !filter.not_senders.is_empty() {
        query = query.filter(event_searches::sender_id.ne_all(&filter.not_senders));
    }
    query
}

fn search_page<'a>(
    room_ids: &'a [OwnedRoomId],
    criteria: &'a Criteria,
    keys: &'a [&'static str],
    cursor: Option<&'a SearchCursor>,
) -> event_searches::BoxedQuery<'a, diesel::pg::Pg> {
    let rank = ts_rank_cd(
        event_searches::vector,
        websearch_to_tsquery(&criteria.search_term),
    );
    let mut query = searchable_events(room_ids, criteria, keys);
    if let Some(cursor) = cursor {
        let older = event_searches::origin_server_ts.lt(cursor.timestamp).or(
            event_searches::origin_server_ts
                .eq(cursor.timestamp)
                .and(event_searches::event_sn.lt(cursor.event_sn)),
        );
        query = if let Some(value) = cursor.rank {
            query.filter(rank.lt(value).or(rank.eq(value).and(older)))
        } else {
            query.filter(older)
        };
        if let Some(room_id) = &cursor.room_id {
            query = query.filter(event_searches::room_id.eq(room_id));
        }
        if let Some(sender) = &cursor.sender {
            query = query.filter(event_searches::sender_id.eq(sender));
        }
    }
    if ranked_search(criteria) {
        query = query.order_by(rank.desc());
    }
    query
        .then_order_by(event_searches::origin_server_ts.desc())
        .then_order_by(event_searches::event_sn.desc())
}

async fn searchable_rooms(user_id: &UserId) -> AppResult<Vec<OwnedRoomId>> {
    // Membership history remains searchable after leaving; visibility is checked per hit.
    Ok(room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("join"))
        .select(room_users::room_id)
        .distinct()
        .load(&mut connect().await?)
        .await?)
}

fn highlights(criteria: &Criteria) -> Vec<String> {
    criteria
        .search_term
        .split_terminator(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .collect()
}

pub async fn search_pdus(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    criteria: &Criteria,
    next_batch: Option<&str>,
) -> AppResult<ResultRoomEvents> {
    let filter = &criteria.filter;
    let ranked = ranked_search(criteria);
    let cursor = next_batch
        .map(|token| SearchCursor::parse(token, ranked))
        .transpose()?;
    let allowed_rooms = searchable_rooms(user_id).await?;

    let mut room_ids = match filter.rooms.clone() {
        Some(rooms) => rooms,
        None => allowed_rooms.clone(),
    };
    room_ids.retain(|room_id| !filter.not_rooms.contains(room_id));

    // Use limit or else 10, with maximum 100
    let limit = filter.limit.unwrap_or(10).min(100);
    if limit == 0 {
        return Err(MatrixError::invalid_param("Search limit must be greater than zero.").into());
    }

    for room_id in &room_ids {
        if !allowed_rooms.contains(room_id) {
            return Err(MatrixError::forbidden(
                "you don't have permission to view this room",
                None,
            )
            .into());
        }
    }

    let keys = searched_keys(criteria);
    if room_ids.is_empty() || keys.is_empty() {
        return Ok(ResultRoomEvents {
            count: Some(0),
            highlights: highlights(criteria),
            ..Default::default()
        });
    }

    let data_query = search_page(&room_ids, criteria, &keys, cursor.as_ref())
        .select((
            ts_rank_cd(
                event_searches::vector,
                websearch_to_tsquery(&criteria.search_term),
            ),
            event_searches::event_id,
            event_searches::event_sn,
            event_searches::origin_server_ts,
        ))
        .limit(limit as i64);
    let items = data_query
        .load::<(f32, OwnedEventId, i64, i64)>(&mut connect().await?)
        .await?;
    let count: i64 = searchable_events(&room_ids, criteria, &keys)
        .count()
        .first(&mut connect().await?)
        .await?;
    let next_batch = if items.len() < limit {
        None
    } else {
        items.last().map(|last| {
            SearchCursor {
                rank: ranked.then_some(last.0),
                timestamp: last.3,
                event_sn: last.2,
                room_id: cursor.as_ref().and_then(|cursor| cursor.room_id.clone()),
                sender: cursor.as_ref().and_then(|cursor| cursor.sender.clone()),
            }
            .encode()
        })
    };

    let mut results = Vec::new();
    let mut hits: Vec<Hit> = Vec::new();
    for (rank, event_id, ..) in items {
        let Ok(pdu) = timeline::get_pdu(&event_id).await else {
            continue;
        };
        if !state::user_can_see_event(user_id, &pdu.event_id)
            .await
            .unwrap_or(false)
        {
            continue;
        }
        hits.push((
            pdu.event_id.clone(),
            pdu.room_id.clone(),
            pdu.sender.clone(),
        ));
        results.push(SearchResult {
            context: calc_event_context(user_id, device_id, &pdu, &criteria.event_context)
                .await
                .unwrap_or_default(),
            rank: Some(rank as f64),
            result: Some(pdu.to_room_event_for(user_id, device_id)),
        });
    }

    let mut room_state = BTreeMap::new();
    if criteria.include_state == Some(true) {
        let result_rooms: BTreeSet<_> = hits.iter().map(|(_, room_id, _)| room_id).collect();
        for room_id in result_rooms {
            let Ok(frame_id) = room::get_frame_id(room_id, None).await else {
                continue;
            };
            let state_events = state::get_full_state(frame_id)
                .await?
                .values()
                .map(|pdu| pdu.to_state_event_for(user_id, device_id))
                .collect();
            room_state.insert(room_id.clone(), state_events);
        }
    }

    Ok(ResultRoomEvents {
        count: Some(count as u64),
        groups: group_hits(&criteria.groupings, &hits, next_batch.as_deref()),
        next_batch,
        results,
        state: room_state,
        highlights: highlights(criteria),
    })
}

/// Partitions the hits by each requested grouping key.
///
/// A group's `order` is the position of its first hit, so that groups sort the same way
/// as the results do.
fn group_hits(
    groupings: &Groupings,
    hits: &[Hit],
    next_batch: Option<&str>,
) -> BTreeMap<GroupingKey, BTreeMap<OwnedRoomIdOrUserId, ResultGroup>> {
    let mut groups = BTreeMap::new();
    for grouping in &groupings.group_by {
        let Some(key) = &grouping.key else {
            continue;
        };
        let group_of = |(_, room_id, sender): &Hit| match key {
            GroupingKey::RoomId => Some(OwnedRoomIdOrUserId::RoomId(room_id.clone())),
            GroupingKey::Sender => Some(OwnedRoomIdOrUserId::UserId(sender.clone())),
            _ => None,
        };
        let groups: &mut BTreeMap<_, ResultGroup> = groups.entry(key.clone()).or_default();
        for hit in hits {
            let Some(group_key) = group_of(hit) else {
                continue;
            };
            let order = groups.len() as u64;
            groups
                .entry(group_key.clone())
                .or_insert_with(|| ResultGroup {
                    next_batch: next_batch.map(|token| {
                        let mut cursor = SearchCursor::parse(token, true)
                            .or_else(|_| SearchCursor::parse(token, false))
                            .expect("server-generated cursor");
                        match &group_key {
                            OwnedRoomIdOrUserId::RoomId(room_id) => {
                                cursor.room_id = Some(room_id.clone())
                            }
                            OwnedRoomIdOrUserId::UserId(sender) => {
                                cursor.sender = Some(sender.clone())
                            }
                        }
                        cursor.encode()
                    }),
                    order: Some(order),
                    results: Vec::new(),
                })
                .results
                .push(hit.0.clone());
        }
    }
    groups
}

// Calculates the contextual events for any search results.
async fn calc_event_context(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    pdu: &SnPduEvent,
    context: &EventContext,
) -> AppResult<EventContextResult> {
    let (before_boundary, after_boundary) = context_boundaries(pdu.event_sn);
    let before_pdus = timeline::stream::load_pdus_backward(
        Some(user_id),
        &pdu.room_id,
        // The stream loader already uses an exclusive boundary. Starting one
        // position earlier skips the event immediately before the search hit.
        Some(before_boundary),
        None,
        None,
        context.before_limit.min(MAX_CONTEXT_LIMIT) as usize,
    )
    .await?;
    let after_pdus = timeline::stream::load_pdus_forward(
        Some(user_id),
        &pdu.room_id,
        // Forward loading includes the supplied stream position, so advance once
        // to exclude the search hit while retaining its immediate successor.
        Some(after_boundary),
        None,
        None,
        context.after_limit.min(MAX_CONTEXT_LIMIT) as usize,
    )
    .await?;

    let profile_info = if context.include_profile {
        let senders: BTreeSet<&UserId> = before_pdus
            .iter()
            .chain(after_pdus.iter())
            .map(|(_, pdu)| pdu.sender.as_ref())
            .chain(std::iter::once(pdu.sender.as_ref()))
            .collect();
        historic_profiles(&pdu.room_id, pdu.event_sn, senders).await
    } else {
        BTreeMap::new()
    };

    let context = EventContextResult {
        start: before_pdus
            .last()
            .map(|(sn, _)| BatchToken::new_live(*sn).to_string()),
        end: after_pdus
            .last()
            .map(|(sn, _)| BatchToken::new_live(*sn + 1).to_string()),
        events_before: before_pdus
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event_for(user_id, device_id))
            .collect(),
        events_after: after_pdus
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event_for(user_id, device_id))
            .collect(),
        profile_info,
    };

    Ok(context)
}

/// The profiles of `senders` as they were at the event: the display name and avatar in
/// their member events in the room state at `event_sn`. Users without a member event
/// there are left out.
async fn historic_profiles(
    room_id: &RoomId,
    event_sn: Seqnum,
    senders: impl IntoIterator<Item = &UserId>,
) -> BTreeMap<OwnedUserId, UserProfile> {
    let mut profiles = BTreeMap::new();
    let Ok(frame_id) = crate::event::get_frame_id(room_id, event_sn).await else {
        return profiles;
    };
    for sender in senders {
        let Ok(RoomMemberEventContent {
            display_name,
            avatar_url,
            ..
        }) = state::get_state_content::<RoomMemberEventContent>(
            frame_id,
            &StateEventType::RoomMember,
            sender.as_str(),
        )
        .await
        else {
            continue;
        };
        profiles.insert(
            sender.to_owned(),
            UserProfile {
                display_name,
                avatar_url,
            },
        );
    }
    profiles
}

fn context_boundaries(event_sn: Seqnum) -> (BatchToken, BatchToken) {
    (
        BatchToken::new_live(event_sn),
        BatchToken::new_live(event_sn + 1),
    )
}

pub async fn save_pdu(pdu: &SnPduEvent, pdu_json: &CanonicalJsonObject) -> AppResult<()> {
    let Some(CanonicalJsonValue::Object(content)) = pdu_json.get("content") else {
        return Ok(());
    };
    let Some((key, vector)) = (match pdu.event_ty {
        TimelineEventType::RoomName => content
            .get("name")
            .and_then(|v| v.as_str())
            .map(|v| ("content.name", v)),
        TimelineEventType::RoomTopic => content
            .get("topic")
            .and_then(|v| v.as_str())
            .map(|v| ("content.topic", v)),
        TimelineEventType::RoomMessage => content
            .get("body")
            .and_then(|v| v.as_str())
            .map(|v| ("content.message", v)),
        // Redaction events themselves have no searchable content. Applying a
        // redaction removes the target from the index in `timeline::redact_pdu`.
        TimelineEventType::RoomRedaction => return Ok(()),
        _ => {
            return Ok(());
        }
    }) else {
        return Ok(());
    };
    diesel::sql_query("INSERT INTO event_searches (event_id, event_sn, room_id, sender_id, key, vector, origin_server_ts) VALUES ($1, $2, $3, $4, $5, to_tsvector('english', $6), $7) ON CONFLICT (event_id) DO UPDATE SET vector = to_tsvector('english', $6), origin_server_ts = $7")
        .bind::<diesel::sql_types::Text, _>(pdu.event_id.as_str())
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Int8>, _>(pdu.event_sn)
        .bind::<diesel::sql_types::Text, _>(&pdu.room_id)
        .bind::<diesel::sql_types::Text, _>(&pdu.sender)
        .bind::<diesel::sql_types::Text, _>(key)
        .bind::<diesel::sql_types::Text, _>(vector)
        .bind::<diesel::sql_types::Int8, _>(pdu.origin_server_ts)
        .bind::<diesel::sql_types::Text, _>(vector)
        .bind::<diesel::sql_types::Int8, _>(pdu.origin_server_ts)
        .execute(&mut connect().await?)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use diesel::debug_query;
    use diesel::pg::Pg;
    use serde_json::json;

    use super::*;

    fn criteria(value: serde_json::Value) -> Criteria {
        serde_json::from_value(value).unwrap()
    }

    fn room(id: &str) -> OwnedRoomId {
        RoomId::parse(id).unwrap().to_owned()
    }

    #[test]
    fn search_query_excludes_redacted_events() {
        let room_ids = vec![room("!room:example.org")];
        let criteria = criteria(json!({"search_term": "needle"}));
        let query = searchable_events(&room_ids, &criteria, &["content.message"]);
        let sql = debug_query::<Pg, _>(&query).to_string();

        assert!(sql.contains("\"events\".\"is_redacted\" ="), "{sql}");
        assert!(
            sql.contains("\"event_searches\".\"event_id\" = ANY(SELECT \"events\".\"id\""),
            "{sql}"
        );
        assert!(!sql.contains("contains_url"), "{sql}");
        assert!(!sql.contains("\"sender_id\" ="), "{sql}");
        assert!(!sql.contains("\"sender_id\" !="), "{sql}");
    }

    #[test]
    fn search_query_applies_the_sender_and_url_filters() {
        let room_ids = vec![room("!room:example.org")];
        let criteria = criteria(json!({
            "search_term": "needle",
            "filter": {
                "senders": ["@alice:example.org"],
                "not_senders": ["@bob:example.org"],
                "contains_url": true
            }
        }));
        let query = searchable_events(&room_ids, &criteria, &["content.message"]);
        let sql = debug_query::<Pg, _>(&query).to_string();

        assert!(sql.contains("\"events\".\"contains_url\" ="), "{sql}");
        assert!(
            sql.contains("\"event_searches\".\"sender_id\" = ANY("),
            "{sql}"
        );
        assert!(
            sql.contains("\"event_searches\".\"sender_id\" != ALL("),
            "{sql}"
        );
        assert!(sql.contains("\"event_searches\".\"key\" = ANY("), "{sql}");
    }

    #[test]
    fn searched_keys_follow_keys_and_type_filters() {
        let all = criteria(json!({"search_term": "x"}));
        assert_eq!(
            searched_keys(&all),
            ["content.message", "content.name", "content.topic"]
        );

        let body_only = criteria(json!({"search_term": "x", "keys": ["content.body"]}));
        assert_eq!(searched_keys(&body_only), ["content.message"]);

        let typed = criteria(json!({
            "search_term": "x",
            "filter": {"types": ["m.room.*"], "not_types": ["m.room.topic"]}
        }));
        assert_eq!(searched_keys(&typed), ["content.message", "content.name"]);

        let unindexed = criteria(json!({
            "search_term": "x",
            "filter": {"types": ["m.room.member"]}
        }));
        assert!(searched_keys(&unindexed).is_empty());
    }

    #[test]
    fn event_type_patterns_support_wildcards() {
        assert!(event_type_matches("m.room.message", "m.room.message"));
        assert!(!event_type_matches(
            "m.room.message",
            "m.room.message.extra"
        ));
        assert!(event_type_matches("m.room.*", "m.room.message"));
        assert!(event_type_matches("*", "m.room.message"));
        assert!(event_type_matches("*.message", "m.room.message"));
        assert!(event_type_matches("m.*.mess*", "m.room.message"));
        assert!(!event_type_matches("m.*.topic", "m.room.message"));
        assert!(!event_type_matches("org.*", "m.room.message"));
    }

    #[test]
    fn hits_are_grouped_by_room_and_sender_in_result_order() {
        let hits: Vec<Hit> = vec![
            (
                EventId::parse("$1").unwrap(),
                room("!a:example.org"),
                UserId::parse("@alice:example.org").unwrap(),
            ),
            (
                EventId::parse("$2").unwrap(),
                room("!b:example.org"),
                UserId::parse("@bob:example.org").unwrap(),
            ),
            (
                EventId::parse("$3").unwrap(),
                room("!a:example.org"),
                UserId::parse("@bob:example.org").unwrap(),
            ),
        ];
        let groupings: Groupings = serde_json::from_value(json!({
            "group_by": [{"key": "room_id"}, {"key": "sender"}]
        }))
        .unwrap();

        let token = SearchCursor {
            rank: None,
            timestamp: 100,
            event_sn: 10,
            room_id: None,
            sender: None,
        }
        .encode();
        let groups = group_hits(&groupings, &hits, Some(&token));

        let rooms = &groups[&GroupingKey::RoomId];
        let room_a = &rooms[&OwnedRoomIdOrUserId::RoomId(room("!a:example.org"))];
        assert_eq!(room_a.order, Some(0));
        assert_eq!(room_a.results, ["$1", "$3"]);
        let cursor = SearchCursor::parse(room_a.next_batch.as_deref().unwrap(), false).unwrap();
        assert_eq!(cursor.room_id.as_ref(), Some(&room("!a:example.org")));
        assert!(cursor.sender.is_none());
        let room_b = &rooms[&OwnedRoomIdOrUserId::RoomId(room("!b:example.org"))];
        assert_eq!(room_b.order, Some(1));
        assert_eq!(room_b.results, ["$2"]);

        let senders = &groups[&GroupingKey::Sender];
        let bob =
            &senders[&OwnedRoomIdOrUserId::UserId(UserId::parse("@bob:example.org").unwrap())];
        assert_eq!(bob.order, Some(1));
        assert_eq!(bob.results, ["$2", "$3"]);

        assert!(group_hits(&Groupings::default(), &hits, None).is_empty());
    }

    #[test]
    fn search_context_respects_the_loaders_boundary_semantics() {
        assert_eq!(
            context_boundaries(42),
            (BatchToken::new_live(42), BatchToken::new_live(43))
        );
    }

    #[test]
    fn search_cursors_validate_order_and_all_fields() {
        let recent = SearchCursor::parse("100-42", false).unwrap();
        assert_eq!(recent.timestamp, 100);
        assert_eq!(recent.event_sn, 42);
        assert!(SearchCursor::parse(&recent.encode(), true).is_err());
        for token in ["", "100", "100-42-extra", "-1-42", "100--1", "s1.invalid"] {
            assert!(SearchCursor::parse(token, false).is_err(), "{token}");
        }
        let ranked = SearchCursor {
            rank: Some(0.12345679),
            ..recent
        };
        assert_eq!(
            SearchCursor::parse(&ranked.encode(), true).unwrap().rank,
            ranked.rank
        );
        assert!(SearchCursor::parse(&ranked.encode(), false).is_err());
        assert!(ranked_search(&Criteria::new("needle".into())));
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_search_pagination_orders_filters_and_scopes_results() {
        use diesel::sql_types::{BigInt, Text};

        crate::test_database::init();
        let room_ids = vec![room("!search:example.org")];
        let mut conn = connect().await.unwrap();
        for (id, sn, ts, text, sender) in [
            ("$search-a", 10, 300, "needle", "@alice:example.org"),
            (
                "$search-b",
                40,
                200,
                "needle needle needle",
                "@bob:example.org",
            ),
            ("$search-c", 30, 200, "needle needle", "@alice:example.org"),
            ("$search-d", 50, 100, "needle", "@bob:example.org"),
        ] {
            diesel::sql_query("INSERT INTO events (id, sn, ty, room_id, topological_ordering, stream_ordering, origin_server_ts, contains_url, is_outlier) VALUES ($1, $2, 'm.room.message', '!search:example.org', $2, $2, $3, false, false)")
                .bind::<Text, _>(id).bind::<BigInt, _>(sn).bind::<BigInt, _>(ts)
                .execute(&mut conn).await.unwrap();
            diesel::sql_query("INSERT INTO event_searches (event_id, event_sn, room_id, sender_id, key, vector, origin_server_ts) VALUES ($1, $2, '!search:example.org', $3, 'content.message', to_tsvector('english', $4), $5)")
                .bind::<Text, _>(id).bind::<BigInt, _>(sn).bind::<Text, _>(sender)
                .bind::<Text, _>(text).bind::<BigInt, _>(ts)
                .execute(&mut conn).await.unwrap();
            let pdu = json!({
                "event_id": id, "room_id": "!search:example.org", "type": "m.room.message",
                "sender": sender, "origin_server_ts": ts, "depth": sn,
                "content": {"body": text, "msgtype": "m.text"}, "hashes": {"sha256": "test"}
            });
            diesel::sql_query("INSERT INTO event_datas (event_id, event_sn, room_id, json_data) VALUES ($1, $2, '!search:example.org', $3)")
                .bind::<Text, _>(id).bind::<BigInt, _>(sn).bind::<diesel::sql_types::Json, _>(pdu)
                .execute(&mut conn).await.unwrap();
        }
        for (order, expected) in [
            (
                Some(OrderBy::Recent),
                vec!["$search-a", "$search-b", "$search-c", "$search-d"],
            ),
            (
                Some(OrderBy::Rank),
                vec!["$search-b", "$search-c", "$search-a", "$search-d"],
            ),
            (
                None,
                vec!["$search-b", "$search-c", "$search-a", "$search-d"],
            ),
        ] {
            let mut criteria = Criteria::new("needle".into());
            criteria.order_by = order;
            let keys = searched_keys(&criteria);
            let mut cursor = None;
            let mut ids = Vec::new();
            loop {
                let page = search_page(&room_ids, &criteria, &keys, cursor.as_ref())
                    .select((
                        ts_rank_cd(
                            event_searches::vector,
                            websearch_to_tsquery(&criteria.search_term),
                        ),
                        event_searches::event_id,
                        event_searches::event_sn,
                        event_searches::origin_server_ts,
                    ))
                    .limit(1)
                    .load::<(f32, OwnedEventId, i64, i64)>(&mut conn)
                    .await
                    .unwrap();
                let Some((rank, id, sn, ts)) = page.as_slice().first() else {
                    break;
                };
                ids.push(id.to_string());
                assert!(ids.len() <= expected.len(), "pagination must make progress");
                cursor = Some(SearchCursor {
                    rank: ranked_search(&criteria).then_some(*rank),
                    timestamp: *ts,
                    event_sn: *sn,
                    room_id: None,
                    sender: None,
                });
                cursor = Some(
                    SearchCursor::parse(&cursor.unwrap().encode(), ranked_search(&criteria))
                        .unwrap(),
                );
            }
            assert_eq!(ids, expected);
        }
        let mut criteria = Criteria::new("needle".into());
        criteria.order_by = Some(OrderBy::Recent);
        let keys = searched_keys(&criteria);
        let cursor = SearchCursor {
            rank: None,
            timestamp: 301,
            event_sn: 0,
            room_id: None,
            sender: Some("@alice:example.org".try_into().unwrap()),
        };
        let scoped = search_page(&room_ids, &criteria, &keys, Some(&cursor))
            .select(event_searches::event_id)
            .load::<OwnedEventId>(&mut conn)
            .await
            .unwrap();
        assert_eq!(scoped, ["$search-a", "$search-c"]);
        // A small context limit returns immediate successors, rather than the room tail.
        let after = timeline::stream::load_pdus_forward(
            None,
            &room_ids[0],
            Some(BatchToken::new_live(11)),
            None,
            None,
            2,
        )
        .await
        .unwrap();
        assert_eq!(
            after
                .values()
                .map(|pdu| pdu.event_id.as_str())
                .collect::<Vec<_>>(),
            ["$search-c", "$search-b"]
        );
        // Each visibility status must exclude an indexed event even if stale index data remains.
        for status in ["is_redacted", "is_rejected", "is_outlier", "soft_failed"] {
            diesel::sql_query(format!(
                "UPDATE events SET {status} = true WHERE id = '$search-a'"
            ))
            .execute(&mut conn)
            .await
            .unwrap();
            let ids = searchable_events(&room_ids, &criteria, &keys)
                .select(event_searches::event_id)
                .load::<OwnedEventId>(&mut conn)
                .await
                .unwrap();
            assert!(!ids.iter().any(|id| id.as_str() == "$search-a"));
            diesel::sql_query(format!(
                "UPDATE events SET {status} = false WHERE id = '$search-a'"
            ))
            .execute(&mut conn)
            .await
            .unwrap();
        }
        diesel::sql_query("INSERT INTO room_users (event_id, event_sn, room_id, user_id, user_server_id, sender_id, membership, created_at) VALUES ('$search-join', 1, '!search:example.org', '@search:example.org', 'example.org', '@search:example.org', 'join', 1), ('$search-leave', 2, '!search:example.org', '@search:example.org', 'example.org', '@search:example.org', 'leave', 2)")
            .execute(&mut conn).await.unwrap();
        let user_id = UserId::parse("@search:example.org").unwrap();
        assert!(
            crate::data::user::joined_rooms(&user_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(searchable_rooms(&user_id).await.unwrap(), room_ids);
    }
}
