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
    let mut start_sn = if dir == Direction::Forward {
        0
    } else {
        data::curr_sn()? + 1
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
        // Note: don't filter by is_outlier here. Events that arrived via federation
        // may still be marked as outliers if some processing didn't complete (e.g.,
        // auth chain resolution), but they should still be visible in sync/messages
        // if they made it into the room. Filter by is_rejected instead.
        let events: Vec<(OwnedEventId, Seqnum)> = if dir == Direction::Forward {
            query
                .filter(events::sn.gt(start_sn))
                .filter(events::is_rejected.eq(false))
                .filter(events::soft_failed.eq(false))
                .order(events::stream_ordering.desc())
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
                .filter(events::soft_failed.eq(false))
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
        start_sn = if dir == Direction::Forward {
            if let Some(sn) = events.iter().map(|(_, sn)| sn).max() {
                *sn
            } else {
                break;
            }
        } else if let Some(sn) = events.iter().map(|(_, sn)| sn).min() {
            *sn
        } else {
            break;
        };
        for (event_id, event_sn) in events {
            if let Ok(mut pdu) = super::get_pdu(&event_id) {
                if let Some(user_id) = user_id {
                    if !pdu.user_can_see(user_id)? {
                        continue;
                    }
                    if pdu.sender != user_id {
                        pdu.remove_transaction_id()?;
                    }
                    pdu.add_unsigned_membership(user_id)?;
                }
                pdu.add_age()?;
                list.insert(event_sn, pdu);
                if list.len() >= limit {
                    break;
                }
            }
        }
    }
    Ok(list)
}
