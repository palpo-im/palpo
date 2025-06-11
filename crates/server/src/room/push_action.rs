use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::iter::once;
use std::sync::{Arc, LazyLock, Mutex};

use diesel::prelude::*;
use palpo_data::schema::events::rejection_reason;
use salvo::server;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use ulid::Ulid;

use crate::core::client::filter::{RoomEventFilter, UrlFilter};
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::encrypted::Relation;
use crate::core::events::room::member::MembershipState;
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::events::{GlobalAccountDataEventType, StateEventType, TimelineEventType};
use crate::core::federation::backfill::BackfillReqArgs;
use crate::core::federation::backfill::{BackfillResBody, backfill_request};
use crate::core::identifiers::*;
use crate::core::presence::PresenceState;
use crate::core::push::{Action, Ruleset, Tweak};
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, JsonValue, RawJsonValue, to_canonical_value};
use crate::core::state::Event;
use crate::core::{Direction, RoomVersion, Seqnum, UnixMillis};
use crate::data::room::{DbEventData, NewDbEvent, NewDbEventPushAction};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::{EventHash, PduBuilder, PduEvent, ensure_event_sn, handler};
use crate::room::state::CompressedState;
use crate::room::{state, timeline};
use crate::{
    AppError, AppResult, GetUrlOrigin, MatrixError, RoomMutexGuard, SnPduEvent, config, data, membership, room, utils,
};

pub fn increment_notification_counts(
    event_id: &EventId,
    notifies: Vec<OwnedUserId>,
    highlights: Vec<OwnedUserId>,
) -> AppResult<()> {
    let (room_id, thread_id) = event_points::table
        .find(event_id)
        .select((event_points::room_id, event_points::thread_id))
        .first::<(OwnedRoomId, Option<OwnedEventId>)>(&mut connect()?)?;

    for user_id in notifies {
        let rows = if let Some(thread_id) = &thread_id {
            diesel::update(
                event_push_summaries::table
                    .filter(event_push_summaries::user_id.eq(&user_id))
                    .filter(event_push_summaries::room_id.eq(&room_id))
                    .filter(event_push_summaries::thread_id.eq(thread_id)),
            )
            .set(event_push_summaries::notification_count.eq(event_push_summaries::notification_count + 1))
            .execute(&mut connect()?)?
        } else {
            diesel::update(
                event_push_summaries::table
                    .filter(event_push_summaries::user_id.eq(&user_id))
                    .filter(event_push_summaries::room_id.eq(&room_id))
                    .filter(event_push_summaries::thread_id.is_null()),
            )
            .set(event_push_summaries::notification_count.eq(event_push_summaries::notification_count + 1))
            .execute(&mut connect()?)?
        };
        if rows == 0 {
            diesel::insert_into(event_push_summaries::table)
                .values((
                    event_push_summaries::user_id.eq(&user_id),
                    event_push_summaries::room_id.eq(&room_id),
                    event_push_summaries::notification_count.eq(1),
                    event_push_summaries::unread_count.eq(1),
                    event_push_summaries::thread_id.eq(&thread_id),
                    event_push_summaries::stream_ordering.eq(1), // TODO: use the correct stream ordering
                ))
                .execute(&mut connect()?)?;
        }
    }
    for user_id in highlights {
        let rows = if let Some(thread_id) = &thread_id {
            diesel::update(
                event_push_summaries::table
                    .filter(event_push_summaries::user_id.eq(&user_id))
                    .filter(event_push_summaries::room_id.eq(&room_id))
                    .filter(event_push_summaries::thread_id.eq(thread_id)),
            )
            .set(event_push_summaries::highlight_count.eq(event_push_summaries::highlight_count + 1))
            .execute(&mut connect()?)?
        } else {
            diesel::update(
                event_push_summaries::table
                    .filter(event_push_summaries::user_id.eq(&user_id))
                    .filter(event_push_summaries::room_id.eq(&room_id))
                    .filter(event_push_summaries::thread_id.is_null()),
            )
            .set(event_push_summaries::highlight_count.eq(event_push_summaries::highlight_count + 1))
            .execute(&mut connect()?)?
        };
        if rows == 0 {
            diesel::insert_into(event_push_summaries::table)
                .values((
                    event_push_summaries::user_id.eq(&user_id),
                    event_push_summaries::room_id.eq(&room_id),
                    event_push_summaries::highlight_count.eq(1),
                    event_push_summaries::unread_count.eq(1),
                    event_push_summaries::thread_id.eq(&thread_id),
                    event_push_summaries::stream_ordering.eq(1), // TODO: use the correct stream ordering
                ))
                .execute(&mut connect()?)?;
        }
    }

    Ok(())
}

#[tracing::instrument]
pub fn upsert_push_action(
    room_id: &RoomId,
    event_id: &EventId,
    user_id: &UserId,
    notify: bool,
    highlight: bool,
) -> AppResult<()> {
    let actions: Vec<String> = vec![];
    let (event_sn, thread_id) = event_points::table
        .find(event_id)
        .select((event_points::event_sn, event_points::thread_id))
        .first::<(Seqnum, Option<OwnedEventId>)>(&mut connect()?)?;
    let (topological_ordering, stream_ordering) = events::table
        .find(event_id)
        .select((events::topological_ordering, events::stream_ordering))
        .first::<(i64, i64)>(&mut connect()?)?;

    data::room::event::upsert_push_action(&NewDbEventPushAction {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        event_sn,
        user_id: user_id.to_owned(),
        profile_tag: "".to_owned(),
        actions: serde_json::to_value(actions).expect("actions is always valid"),
        topological_ordering,
        stream_ordering,
        notify,
        highlight,
        unread: false,
        thread_id,
    })?;

    Ok(())
}

pub fn remove_actions_until(
    user_id: &UserId,
    room_id: &RoomId,
    event_sn: Seqnum,
    thread_id: Option<&EventId>,
) -> AppResult<()> {
    if let Some(thread_id) = thread_id {
        diesel::delete(
            event_push_actions::table
                .filter(event_push_actions::user_id.eq(user_id))
                .filter(event_push_actions::room_id.eq(room_id))
                .filter(event_push_actions::thread_id.eq(thread_id))
                .filter(event_push_actions::event_sn.le(event_sn)),
        )
        .execute(&mut connect()?)?;
    } else {
        diesel::delete(
            event_push_actions::table
                .filter(event_push_actions::user_id.eq(user_id))
                .filter(event_push_actions::room_id.eq(room_id))
                .filter(event_push_actions::event_sn.le(event_sn)),
        )
        .execute(&mut connect()?)?;
    }
    Ok(())
}

pub fn remove_actions_for_room(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    diesel::delete(
        event_push_actions::table
            .filter(event_push_actions::user_id.eq(user_id))
            .filter(event_push_actions::room_id.eq(room_id)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

pub fn refresh_notify_summary(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    let thread_ids = event_push_actions::table
        .filter(event_push_actions::user_id.eq(user_id))
        .filter(event_push_actions::room_id.eq(room_id))
        .select(event_push_actions::thread_id)
        .distinct()
        .load::<Option<OwnedEventId>>(&mut connect()?)?
        .into_iter()
        .filter_map(|x| x)
        .collect::<Vec<_>>();
    diesel::delete(
        event_push_actions::table
            .filter(event_push_actions::user_id.eq(user_id))
            .filter(event_push_actions::room_id.eq(room_id))
            .filter(event_push_actions::thread_id.is_not_null())
            .filter(event_push_actions::thread_id.ne_all(&thread_ids)),
    )
    .execute(&mut connect()?)?;
    diesel::delete(
        event_push_summaries::table
            .filter(event_push_summaries::user_id.eq(user_id))
            .filter(event_push_summaries::room_id.eq(room_id))
            .filter(event_push_summaries::thread_id.is_not_null())
            .filter(event_push_summaries::thread_id.ne_all(&thread_ids)),
    )
    .execute(&mut connect()?)?;
    for thread_id in &thread_ids {
        let query = event_push_actions::table
            .filter(event_push_actions::user_id.eq(user_id))
            .filter(event_push_actions::room_id.eq(room_id))
            .filter(event_push_actions::thread_id.eq(thread_id));
        let notification_count = query
            .clone()
            .filter(event_push_actions::notify.eq(true))
            .count()
            .get_result::<i64>(&mut connect()?)?;
        let highlight_count = query
            .clone()
            .filter(event_push_actions::highlight.eq(true))
            .count()
            .get_result::<i64>(&mut connect()?)?;
        let unread_count = query
            .clone()
            .filter(event_push_actions::unread.eq(true))
            .count()
            .get_result::<i64>(&mut connect()?)?;

        let rows = diesel::update(
            event_push_summaries::table
                .filter(event_push_summaries::user_id.eq(&user_id))
                .filter(event_push_summaries::room_id.eq(&room_id))
                .filter(event_push_summaries::thread_id.eq(thread_id)),
        )
        .set((
            event_push_summaries::notification_count.eq(notification_count),
            event_push_summaries::highlight_count.eq(highlight_count),
            event_push_summaries::unread_count.eq(unread_count),
        ))
        .execute(&mut connect()?)?;
        if rows == 0 {
            diesel::insert_into(event_push_summaries::table)
                .values((
                    event_push_summaries::user_id.eq(&user_id),
                    event_push_summaries::room_id.eq(&room_id),
                    event_push_summaries::thread_id.eq(thread_id),
                    event_push_summaries::notification_count.eq(notification_count),
                    event_push_summaries::highlight_count.eq(highlight_count),
                    event_push_summaries::unread_count.eq(unread_count),
                    event_push_summaries::stream_ordering.eq(1), // TODO: use the correct stream ordering
                ))
                .execute(&mut connect()?)?;
        }
    }

    let query = event_push_actions::table
        .filter(event_push_actions::user_id.eq(user_id))
        .filter(event_push_actions::room_id.eq(room_id))
        .filter(event_push_actions::thread_id.is_null());
    let notification_count = query
        .clone()
        .filter(event_push_actions::notify.eq(true))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let highlight_count = query
        .clone()
        .filter(event_push_actions::highlight.eq(true))
        .count()
        .get_result::<i64>(&mut connect()?)?;
    let unread_count = query
        .clone()
        .filter(event_push_actions::unread.eq(true))
        .count()
        .get_result::<i64>(&mut connect()?)?;

    let rows = diesel::update(
        event_push_summaries::table
            .filter(event_push_summaries::user_id.eq(&user_id))
            .filter(event_push_summaries::room_id.eq(&room_id))
            .filter(event_push_summaries::thread_id.is_null()),
    )
    .set((
        event_push_summaries::notification_count.eq(notification_count),
        event_push_summaries::highlight_count.eq(highlight_count),
        event_push_summaries::unread_count.eq(unread_count),
    ))
    .execute(&mut connect()?)?;
    if rows == 0 {
        diesel::insert_into(event_push_summaries::table)
            .values((
                event_push_summaries::user_id.eq(&user_id),
                event_push_summaries::room_id.eq(&room_id),
                event_push_summaries::notification_count.eq(notification_count),
                event_push_summaries::highlight_count.eq(highlight_count),
                event_push_summaries::unread_count.eq(unread_count),
                event_push_summaries::stream_ordering.eq(1), // TODO: use the correct stream ordering
            ))
            .execute(&mut connect()?)?;
    }
    Ok(())
}
