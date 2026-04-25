use diesel::prelude::*;
use indexmap::IndexMap;

use crate::core::client::filter::{RoomEventFilter, UrlFilter};
use crate::core::identifiers::*;
use crate::core::{Direction, Seqnum};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::BatchToken;
use crate::{AppResult, SnPduEvent, data, utils};

/// Returns an iterator over all PDUs in a room.
pub fn load_all_pdus(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    until_tk: Option<BatchToken>,
) -> AppResult<IndexMap<i64, SnPduEvent>> {
    load_pdus_forward(user_id, room_id, None, until_tk, None, usize::MAX)
}

pub fn load_pdus_forward(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    filter: Option<&RoomEventFilter>,
    limit: usize,
) -> AppResult<IndexMap<i64, SnPduEvent>> {
    load_pdus(
        user_id,
        room_id,
        since_tk,
        until_tk,
        limit,
        filter,
        Direction::Forward,
    )
}
pub fn load_pdus_backward(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    filter: Option<&RoomEventFilter>,
    limit: usize,
) -> AppResult<IndexMap<i64, SnPduEvent>> {
    load_pdus(
        user_id,
        room_id,
        since_tk,
        until_tk,
        limit,
        filter,
        Direction::Backward,
    )
}

/// Returns an iterator over all events and their tokens in a room that happened before the
/// event with id `until` in reverse-chronological order.
/// Skips events before user joined the room.
#[tracing::instrument]
pub fn load_pdus(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    limit: usize,
    filter: Option<&RoomEventFilter>,
    dir: Direction,
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    let mut list: IndexMap<Seqnum, SnPduEvent> = IndexMap::with_capacity(limit.clamp(10, 100));
    let ignored_users = user_id.map(crate::user::ignored_users);
    let mut offset = 0;
    let mut start_sn = if dir == Direction::Forward {
        0
    } else {
        // Use the max sn observed in the events table for this room as a safer
        // upper bound than data::curr_sn() (which can be stale relative to writes
        // from other sessions when sequence caching is enabled).
        let max_in_room: Option<Seqnum> = events::table
            .filter(events::room_id.eq(room_id))
            .select(diesel::dsl::max(events::sn))
            .first::<Option<Seqnum>>(&mut connect()?)?;
        max_in_room
            .map(|sn| sn + 1)
            .unwrap_or_else(|| data::curr_sn().unwrap_or(0) + 1)
    };

    while list.len() < limit {
        let mut query = events::table
            .filter(events::room_id.eq(room_id))
            .into_boxed();
        if dir == Direction::Forward {
            if let Some(since_tk) = since_tk {
                query = query.filter(events::stream_ordering.ge(since_tk.stream_ordering()));
            }
            if let Some(until_tk) = until_tk {
                query = query.filter(events::stream_ordering.lt(until_tk.stream_ordering()));
            }
        } else {
            if let Some(since_tk) = since_tk {
                query = query.filter(events::stream_ordering.lt(since_tk.stream_ordering()));
            }
            if let Some(until_tk) = until_tk {
                query = query.filter(events::stream_ordering.ge(until_tk.stream_ordering()));
            }
        }

        if let Some(filter) = filter {
            if let Some(url_filter) = &filter.url_filter {
                match url_filter {
                    UrlFilter::EventsWithUrl => query = query.filter(events::contains_url.eq(true)),
                    UrlFilter::EventsWithoutUrl => {
                        query = query.filter(events::contains_url.eq(false))
                    }
                }
            }
            if !filter.not_types.is_empty() {
                query = query.filter(events::ty.ne_all(&filter.not_types));
            }
            if !filter.not_rooms.is_empty() {
                query = query.filter(events::room_id.ne_all(&filter.not_rooms));
            }
            if let Some(rooms) = &filter.rooms
                && !rooms.is_empty()
            {
                query = query.filter(events::room_id.eq_any(rooms));
            }
            if let Some(senders) = &filter.senders
                && !senders.is_empty()
            {
                query = query.filter(events::sender_id.eq_any(senders));
            }
            if let Some(types) = &filter.types
                && !types.is_empty()
            {
                query = query.filter(events::ty.eq_any(types));
            }
        }
        // Don't filter by is_outlier or soft_failed here:
        //  - Federation events that arrive with missing prev_events end up marked as outlier and/or
        //    soft_failed even though they should still be visible to clients (per Matrix spec,
        //    soft-failed events appear in the timeline; they just don't contribute to room state).
        //  - We *do* filter is_rejected because rejected events should not be visible at all.
        let events: Vec<(OwnedEventId, Seqnum)> = if dir == Direction::Forward {
            query
                .filter(events::sn.gt(start_sn))
                .filter(events::is_rejected.eq(false))
                .order(events::stream_ordering.desc())
                .offset(offset)
                .limit(utils::usize_to_i64(limit))
                .select((events::id, events::sn))
                .load::<(OwnedEventId, Seqnum)>(&mut connect()?)?
                .into_iter()
                .rev()
                .collect()
        } else {
            query
                .filter(events::sn.lt(start_sn))
                .filter(events::is_rejected.eq(false))
                .order(events::sn.desc())
                .limit(utils::usize_to_i64(limit))
                .select((events::id, events::sn))
                .load::<(OwnedEventId, Seqnum)>(&mut connect()?)?
                .into_iter()
                .collect()
        };
        if events.is_empty() {
            break;
        }
        let count = events.len();
        if dir == Direction::Forward {
            offset += count as i64;
        } else {
            start_sn = if let Some(sn) = events.iter().map(|(_, sn)| sn).min() {
                *sn
            } else {
                break;
            };
        }
        for (event_id, event_sn) in events {
            match super::get_pdu(&event_id) {
                Ok(mut pdu) => {
                    if let Some(user_id) = user_id
                        && !pdu.user_can_see(user_id).unwrap_or(false)
                    {
                        continue;
                    }
                    if let Some(ignored_users) = &ignored_users
                        && crate::event::is_ignored_pdu_by_ignored_users(&pdu, ignored_users)
                    {
                        continue;
                    }
                    if let Some(user_id) = user_id {
                        if pdu.sender != user_id {
                            pdu.remove_transaction_id()?;
                        }
                        let _ = pdu.add_unsigned_membership(user_id);
                    }
                    let _ = pdu.add_age();
                    list.insert(event_sn, pdu);
                    if list.len() >= limit {
                        break;
                    }
                }
                Err(e) => {
                    warn!(
                        "load_pdus: failed to get_pdu for event {} sn={} in room {}: {}",
                        event_id, event_sn, room_id, e
                    );
                }
            }
        }
        if dir == Direction::Forward && count < limit {
            break;
        }
    }
    Ok(list)
}
