use std::collections::BTreeMap;

use diesel::prelude::*;
use palpo_core::Seqnum;

use crate::core::client::search::{
    Criteria, EventContextResult, OrderBy, ResultRoomEvents, SearchResult,
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
use crate::{AppResult, MatrixError, SnPduEvent};

pub fn search_pdus(
    user_id: &UserId,
    criteria: &Criteria,
    next_batch: Option<&str>,
) -> AppResult<ResultRoomEvents> {
    let filter = &criteria.filter;

    let room_ids = filter
        .rooms
        .clone()
        .unwrap_or_else(|| data::user::joined_rooms(user_id).unwrap_or_default());

    // Use limit or else 10, with maximum 100
    let limit = filter.limit.unwrap_or(10).min(100);

    for room_id in &room_ids {
        if !crate::room::user::is_joined(user_id, room_id)? {
            return Err(MatrixError::forbidden(
                "you don't have permission to view this room",
                None,
            )
            .into());
        }
    }

    let base_query = event_searches::table
        .filter(event_searches::room_id.eq_any(&room_ids))
        .filter(event_searches::vector.matches(websearch_to_tsquery(&criteria.search_term)));
    let mut data_query = base_query.clone().into_boxed();
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
            // event_searches::room_id,
            event_searches::event_id,
            event_searches::event_sn,
            event_searches::origin_server_ts,
            // event_searches::stream_ordering,
        ))
        .limit(limit as i64);
    let items = if criteria.order_by == Some(OrderBy::Rank) {
        data_query
            .order_by(diesel::dsl::sql::<diesel::sql_types::Int8>("1"))
            .load::<(f32, OwnedEventId, i64, i64)>(&mut connect()?)?
    } else {
        data_query
            .order_by(event_searches::origin_server_ts.desc())
            .then_order_by(event_searches::event_sn.desc())
            .load::<(f32, OwnedEventId, i64, i64)>(&mut connect()?)?
    };
    // let _ids: Vec<i64> = event_searches::table
    //     .select(event_searches::id)
    //     .load(&mut connect()?)?;
    let count: i64 = base_query.count().first(&mut connect()?)?;
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

    let results: Vec<_> = items
        .into_iter()
        .filter_map(|(rank, event_id, ..)| {
            let pdu = timeline::get_pdu(&event_id).ok()?;
            if state::user_can_see_event(user_id, &pdu.event_id).unwrap_or(false) {
                Some((rank, pdu))
            } else {
                None
            }
        })
        .map(|(rank, pdu)| SearchResult {
            context: calc_event_context(user_id, &pdu.room_id, pdu.event_sn, 10, 10, false)
                .unwrap_or_default(),
            rank: Some(rank as f64),
            result: Some(pdu.to_room_event()),
        })
        .collect();

    Ok(ResultRoomEvents {
        count: Some(count as u64),
        groups: BTreeMap::new(), // TODO
        next_batch,
        results,
        state: BTreeMap::new(), // TODO
        highlights: criteria
            .search_term
            .split_terminator(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .collect(),
    })
}

// Calculates the contextual events for any search results.
fn calc_event_context(
    user_id: &UserId,
    room_id: &RoomId,
    event_sn: Seqnum,
    before_limit: usize,
    after_limit: usize,
    include_profile: bool,
) -> AppResult<EventContextResult> {
    let before_pdus = timeline::stream::load_pdus_backward(
        Some(user_id),
        room_id,
        Some(BatchToken::new_live(event_sn - 1)),
        None,
        None,
        before_limit,
    )?;
    let after_pdus = timeline::stream::load_pdus_forward(
        Some(user_id),
        room_id,
        Some(BatchToken::new_live(event_sn + 1)),
        None,
        None,
        after_limit,
    )?;
    let mut profile = BTreeMap::new();
    if include_profile && let Ok(frame_id) = crate::event::get_frame_id(room_id, event_sn) {
        let RoomMemberEventContent {
            display_name,
            avatar_url,
            ..
        } = state::get_state_content(frame_id, &StateEventType::RoomMember, user_id.as_str())?;
        if let Some(display_name) = display_name {
            profile.insert("displayname".to_string(), display_name);
        }
        if let Some(avatar_url) = avatar_url {
            profile.insert("avatar_url".to_string(), avatar_url.to_string());
        }
    }

    let context = EventContextResult {
        start: before_pdus.first().map(|(sn, _)| sn.to_string()),
        end: after_pdus.last().map(|(sn, _)| sn.to_string()),
        events_before: before_pdus
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event())
            .collect(),
        events_after: after_pdus
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event())
            .collect(),
        profile_info: BTreeMap::new(),
    };

    Ok(context)
}

pub fn save_pdu(pdu: &SnPduEvent, pdu_json: &CanonicalJsonObject) -> AppResult<()> {
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
        TimelineEventType::RoomRedaction => {
            // TODO: Redaction
            return Ok(());
        }
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
        .execute(&mut connect()?)?;

    Ok(())
}
