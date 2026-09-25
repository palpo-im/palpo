use std::collections::{BTreeMap, BTreeSet};

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
use crate::data::full_text_search::*;
use crate::data::schema::*;
use crate::data::{self, connect};
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

    let mut room_ids = match filter.rooms.clone() {
        Some(rooms) => rooms,
        None => data::user::joined_rooms(user_id).await.unwrap_or_default(),
    };
    room_ids.retain(|room_id| !filter.not_rooms.contains(room_id));

    // Use limit or else 10, with maximum 100
    let limit = filter.limit.unwrap_or(10).min(100);

    for room_id in &room_ids {
        if !crate::room::user::is_joined(user_id, room_id).await? {
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

    let mut data_query = searchable_events(&room_ids, criteria, &keys);
    if let Some(mut next_batch) = next_batch.map(|nb| nb.split('-')) {
        let server_ts: i64 = next_batch.next().map(str::parse).transpose()?.unwrap_or(0);
        let event_sn: i64 = next_batch.next().map(str::parse).transpose()?.unwrap_or(0);
        data_query = data_query
            .filter(event_searches::origin_server_ts.le(server_ts))
            .filter(event_searches::event_sn.lt(event_sn));
    }
    let data_query = data_query
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
    let items = if criteria.order_by == Some(OrderBy::Rank) {
        data_query
            .order_by(diesel::dsl::sql::<diesel::sql_types::Int8>("1"))
            .load::<(f32, OwnedEventId, i64, i64)>(&mut connect().await?)
            .await?
    } else {
        data_query
            .order_by(event_searches::origin_server_ts.desc())
            .then_order_by(event_searches::event_sn.desc())
            .load::<(f32, OwnedEventId, i64, i64)>(&mut connect().await?)
            .await?
    };
    let count: i64 = searchable_events(&room_ids, criteria, &keys)
        .count()
        .first(&mut connect().await?)
        .await?;
    let next_batch = if items.len() < limit {
        None
    } else if let Some(last) = items.last() {
        if criteria.order_by == Some(OrderBy::Recent) || criteria.order_by.is_none() {
            Some(format!("{}-{}", last.3, last.2))
        } else {
            None
        }
    } else {
        None
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
                .entry(group_key)
                .or_insert_with(|| ResultGroup {
                    next_batch: next_batch.map(ToOwned::to_owned),
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
            .iter()
            .next()
            .map(|(sn, _)| BatchToken::new_live(*sn).to_string()),
        end: after_pdus
            .last()
            .map(|(sn, _)| BatchToken::new_live(*sn).to_string()),
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

        let groups = group_hits(&groupings, &hits, Some("token"));

        let rooms = &groups[&GroupingKey::RoomId];
        let room_a = &rooms[&OwnedRoomIdOrUserId::RoomId(room("!a:example.org"))];
        assert_eq!(room_a.order, Some(0));
        assert_eq!(room_a.results, ["$1", "$3"]);
        assert_eq!(room_a.next_batch.as_deref(), Some("token"));
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
}
