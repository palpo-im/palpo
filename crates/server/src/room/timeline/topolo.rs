use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use indexmap::IndexMap;

use crate::core::client::filter::{RoomEventFilter, UrlFilter};
use crate::core::identifiers::*;
use crate::core::{Direction, Seqnum};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::BatchToken;
use crate::{AppResult, SnPduEvent, utils};

pub async fn load_pdus_forward(
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
    .await
}
pub async fn load_pdus_backward(
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
    .await
}

/// A captured sync may recover negative history, but not a live event which
/// was only published after its upper cursor, regardless of the cached identity.
pub async fn load_pdus_backward_captured(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    before: BatchToken,
    filter: Option<&RoomEventFilter>,
    limit: usize,
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    load_pdus_selected(
        user_id,
        room_id,
        None,
        None,
        limit,
        filter,
        (
            Direction::Backward,
            Some(super::publication::Window {
                lower: None,
                upper: Some(before.event_sn()),
            }),
        ),
    )
    .await
}

/// Returns events and their tokens in the requested pagination direction.
/// Forward pages are chronological; backward pages are reverse chronological.
/// Skips events before user joined the room.
#[tracing::instrument]
pub async fn load_pdus(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    limit: usize,
    filter: Option<&RoomEventFilter>,
    dir: Direction,
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    load_pdus_selected(
        user_id,
        room_id,
        since_tk,
        until_tk,
        limit,
        filter,
        (dir, None),
    )
    .await
}

/// Retain historical ordering and boundaries inside a frozen publication window.
pub(crate) async fn load_publication_window(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    limit: usize,
    filter: Option<&RoomEventFilter>,
    selection: (Direction, super::publication::Window),
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    load_pdus_selected(
        user_id,
        room_id,
        since_tk,
        until_tk,
        limit,
        filter,
        (selection.0, Some(selection.1)),
    )
    .await
}

async fn load_pdus_selected(
    user_id: Option<&UserId>,
    room_id: &RoomId,
    since_tk: Option<BatchToken>,
    until_tk: Option<BatchToken>,
    limit: usize,
    filter: Option<&RoomEventFilter>,
    (dir, publication_window): (Direction, Option<super::publication::Window>),
) -> AppResult<IndexMap<Seqnum, SnPduEvent>> {
    if since_tk
        .into_iter()
        .chain(until_tk)
        .any(super::publication::uses_publication)
    {
        return Err(crate::MatrixError::invalid_param(
            "publication cursor requires a qualified reader",
        )
        .into());
    }
    let mut list: IndexMap<Seqnum, SnPduEvent> = IndexMap::with_capacity(limit.clamp(10, 100));
    let ignored_users = match user_id {
        Some(user_id) => Some(crate::user::ignored_users(user_id).await),
        None => None,
    };
    let mut offset = 0;
    while list.len() < limit {
        let mut query = events::table
            .filter(events::room_id.eq(room_id))
            .filter(events::is_outlier.eq(false))
            .filter(events::soft_failed.eq(false))
            .into_boxed();

        if let Some(window) = publication_window {
            let mut published = event_points::table
                .filter(event_points::room_id.eq(room_id))
                .filter(event_points::live_published_sn.is_not_null())
                .select(event_points::event_id)
                .into_boxed();
            if let Some(lower) = window.lower {
                published = published.filter(event_points::live_published_sn.ge(lower));
            }
            if let Some(upper) = window.upper {
                published = published.filter(event_points::live_published_sn.lt(upper));
            }
            query = query.filter(
                events::stream_ordering
                    .ge(0)
                    .and(events::id.eq_any(published))
                    .or(events::stream_ordering.lt(0).and(diesel::dsl::sql::<
                        diesel::sql_types::Bool,
                    >(
                        if window.lower.is_none() {
                            "TRUE"
                        } else {
                            "FALSE"
                        },
                    ))),
            );
        }

        // Live sync tokens contain the next stream position, so their bounds form a
        // half-open interval. Historic tokens identify the event immediately before
        // the position, so forward pagination excludes `since` but includes `until`;
        // backward pagination uses the inverse boundaries.
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
                                    .and(events::stream_ordering.gt(stream_ordering)),
                            ),
                        );
                    }
                    _ => {
                        return Err(crate::MatrixError::invalid_param(
                            "publication cursor requires a qualified reader",
                        )
                        .into());
                    }
                }
            }
            if let Some(until_tk) = until_tk {
                match until_tk {
                    BatchToken::Live { stream_ordering } => {
                        query = query.filter(events::stream_ordering.lt(stream_ordering));
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
                    _ => {
                        return Err(crate::MatrixError::invalid_param(
                            "publication cursor requires a qualified reader",
                        )
                        .into());
                    }
                }
            }
        } else {
            if let Some(since_tk) = since_tk {
                match since_tk {
                    BatchToken::Live { stream_ordering } => {
                        query = query.filter(events::stream_ordering.lt(stream_ordering));
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
                    _ => {
                        return Err(crate::MatrixError::invalid_param(
                            "publication cursor requires a qualified reader",
                        )
                        .into());
                    }
                }
            }
            if let Some(until_tk) = until_tk {
                if let Some(topological_ordering) = until_tk.topological_ordering() {
                    query = query.filter(
                        events::topological_ordering.gt(topological_ordering).or(
                            events::topological_ordering
                                .eq(topological_ordering)
                                .and(events::stream_ordering.gt(until_tk.stream_ordering())),
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
                .filter(events::is_rejected.eq(false))
                .order((
                    events::topological_ordering.asc(),
                    events::stream_ordering.asc(),
                ))
                .offset(offset)
                .limit(utils::usize_to_i64(limit))
                .select((events::id, events::sn, events::stream_ordering))
                .load::<(OwnedEventId, Seqnum, i64)>(&mut connect().await?)
                .await?
                .into_iter()
                .collect()
        } else {
            query = query
                .filter(events::is_rejected.eq(false))
                .order((
                    events::topological_ordering.desc(),
                    events::stream_ordering.desc(),
                ))
                .offset(offset)
                .limit(utils::usize_to_i64(limit));
            query
                .select((events::id, events::sn, events::stream_ordering))
                .load::<(OwnedEventId, Seqnum, i64)>(&mut connect().await?)
                .await?
                .into_iter()
                .collect()
        };
        if events.is_empty() {
            break;
        }
        let count = events.len();
        offset += count as i64;

        for (event_id, event_sn, _) in events {
            let loaded = match super::get_pdu(&event_id).await {
                Err(error) if publication_window.is_some() && !error.is_not_found() => {
                    return Err(error);
                }
                result => result,
            };
            if let Ok(mut pdu) = loaded {
                if let Some(user_id) = user_id {
                    if !pdu.user_can_see(user_id).await? {
                        continue;
                    }
                    if let Some(ignored_users) = &ignored_users
                        && crate::event::is_ignored_pdu_by_ignored_users(&pdu, ignored_users)
                    {
                        continue;
                    }
                    if pdu.sender != user_id {
                        pdu.remove_sender_only_unsigned()?;
                    }
                    pdu.add_unsigned_membership(user_id).await?;
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
