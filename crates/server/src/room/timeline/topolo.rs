use diesel::prelude::*;
use indexmap::IndexMap;

use crate::core::client::filter::{RoomEventFilter, UrlFilter};
use crate::core::identifiers::*;
use crate::core::{Direction, Seqnum};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::BatchToken;
use crate::{AppResult, SnPduEvent, utils};

pub fn load_pdus_forward(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    filter: Option<&RoomEventFilter>,
    limit: usize,
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
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
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
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
    let mut offset = 0;
    while list.len() < limit {
        let mut query = events::table
            .filter(events::room_id.eq(room_id))
            .into_boxed();
        if dir == Direction::Forward {
            if let Some(since_tk) = since_tk {
                match since_tk {
                    BatchToken::Live { stream_ordering } => {
                        query = query.filter(events::stream_ordering.ge(stream_ordering));
                    }
                    BatchToken::Historic {
                        topological_ordering,
                        stream_ordering,
                    } => {
                        query = query.filter(
                            events::topological_ordering.gt(topological_ordering).or(
                                events::topological_ordering
                                    .eq(topological_ordering)
                                    .and(events::stream_ordering.ge(stream_ordering)),
                            ),
                        );
                    }
                }
            }
            if let Some(until_tk) = until_tk {
                match until_tk {
                    BatchToken::Live { stream_ordering } => {
                        query = query.filter(events::stream_ordering.le(stream_ordering));
                    }
                    BatchToken::Historic {
                        topological_ordering,
                        stream_ordering,
                    } => {
                        query = query.filter(
                            events::topological_ordering.lt(topological_ordering).or(
                                events::topological_ordering
                                    .eq(topological_ordering)
                                    .and(events::stream_ordering.le(stream_ordering)),
                            ),
                        );
                    }
                }
            }
        } else {
            if let Some(since_tk) = since_tk {
                match since_tk {
                    BatchToken::Live { stream_ordering } => {
                        query = query.filter(events::stream_ordering.le(stream_ordering));
                    }
                    BatchToken::Historic {
                        topological_ordering,
                        stream_ordering,
                    } => {
                        query = query.filter(
                            events::topological_ordering.lt(topological_ordering).or(
                                events::topological_ordering
                                    .eq(topological_ordering)
                                    .and(events::stream_ordering.le(stream_ordering)),
                            ),
                        );
                    }
                }
            }
            if let Some(until_tk) = until_tk {
                if let Some(topological_ordering) = until_tk.topological_ordering() {
                    query = query.filter(
                        events::topological_ordering.gt(topological_ordering).or(
                            events::topological_ordering
                                .eq(topological_ordering)
                                .and(events::stream_ordering.ge(until_tk.stream_ordering())),
                        ),
                    );
                } else {
                    query = query.filter(events::stream_ordering.ge(until_tk.stream_ordering()));
                }
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
        let events: Vec<(OwnedEventId, Seqnum, i64)> = if dir == Direction::Forward {
            query
                .order((
                    events::topological_ordering.asc(),
                    events::origin_server_ts.asc(),
                    events::stream_ordering.asc(),
                ))
                .offset(offset)
                .limit(utils::usize_to_i64(limit))
                .select((events::id, events::sn, events::stream_ordering))
                .load::<(OwnedEventId, Seqnum, i64)>(&mut connect()?)?
                .into_iter()
                .rev()
                .collect()
        } else {
            query = query
                .order((
                    events::topological_ordering.desc(),
                    events::origin_server_ts.desc(),
                    events::stream_ordering.desc(),
                ))
                .offset(offset)
                .limit(utils::usize_to_i64(limit));
            query
                .select((events::id, events::sn, events::stream_ordering))
                .load::<(OwnedEventId, Seqnum, i64)>(&mut connect()?)?
                .into_iter()
                .collect()
        };
        if events.is_empty() {
            break;
        }
        let count = events.len();
        offset += count as i64;

        for (event_id, event_sn, _) in events {
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
        if count < limit {
            break;
        }
    }
    Ok(list)
}
