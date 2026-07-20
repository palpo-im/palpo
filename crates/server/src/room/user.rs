use std::collections::{BTreeMap, HashMap, HashSet};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use indexmap::IndexMap;

use crate::core::Seqnum;
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::events::room::member::MembershipState;
use crate::core::events::{AnyStrippedStateEvent, AnySyncStateEvent, GlobalAccountDataEventType};
use crate::core::identifiers::*;
use crate::core::push::{AnyPushRuleRef, NewPushRule, NewSimplePushRule};
use crate::core::serde::{JsonValue, RawJson};
use crate::data::room::{DbEventPushSummary, DbRoomTag, NewDbRoomTag};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::BatchToken;
use crate::exts::*;
use crate::{AppResult, MatrixError, utils};

#[derive(Debug, Clone)]
pub struct UserNotifySummary {
    pub notification_count: u64,
    pub unread_count: u64,
    pub highlight_count: u64,

    pub threads: BTreeMap<OwnedEventId, ThreadPushSummary>,
}

#[derive(Debug, Clone)]
pub struct ThreadPushSummary {
    pub notification_count: u64,
    pub unread_count: u64,
    pub highlight_count: u64,
}

impl UserNotifySummary {
    pub fn all_notification_count(&self) -> u64 {
        self.notification_count
            + self
                .threads
                .values()
                .map(|t| t.notification_count)
                .sum::<u64>()
    }
    pub fn all_unread_count(&self) -> u64 {
        self.notification_count + self.threads.values().map(|t| t.unread_count).sum::<u64>()
    }
    pub fn all_highlight_count(&self) -> u64 {
        self.highlight_count
            + self
                .threads
                .values()
                .map(|t| t.highlight_count)
                .sum::<u64>()
    }
}

impl From<Vec<DbEventPushSummary>> for UserNotifySummary {
    fn from(summaries: Vec<DbEventPushSummary>) -> Self {
        let mut notification_count = 0;
        let mut unread_count = 0;
        let mut highlight_count = 0;
        let mut threads = BTreeMap::new();

        for summary in summaries {
            if let Some(thread_id) = summary.thread_id {
                threads.insert(
                    thread_id,
                    ThreadPushSummary {
                        notification_count: summary.notification_count as u64,
                        unread_count: summary.unread_count as u64,
                        highlight_count: summary.highlight_count as u64,
                    },
                );
            } else {
                notification_count += summary.notification_count as u64;
                unread_count += summary.unread_count as u64;
                highlight_count += summary.highlight_count as u64;
            }
        }

        UserNotifySummary {
            notification_count,
            unread_count,
            highlight_count,
            threads,
        }
    }
}

pub async fn notify_summary(user_id: &UserId, room_id: &RoomId) -> AppResult<UserNotifySummary> {
    let summaries = event_push_summaries::table
        .filter(event_push_summaries::user_id.eq(user_id))
        .filter(event_push_summaries::room_id.eq(room_id))
        .load::<DbEventPushSummary>(&mut connect().await?)
        .await?;
    Ok(summaries.into())
}

pub async fn highlight_count(user_id: &UserId, room_id: &RoomId) -> AppResult<u64> {
    let count = event_push_summaries::table
        .filter(event_push_summaries::user_id.eq(user_id))
        .filter(event_push_summaries::room_id.eq(room_id))
        .select(event_push_summaries::highlight_count)
        .first::<i64>(&mut connect().await?)
        .await
        .unwrap_or_default();
    Ok(count as u64)
}

pub async fn last_read_notification(user_id: &UserId, room_id: &RoomId) -> AppResult<Seqnum> {
    let sn = event_receipts::table
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::room_id.eq(room_id))
        .order_by(event_receipts::event_sn.desc())
        .select(event_receipts::event_sn)
        .first::<Seqnum>(&mut connect().await?)
        .await
        .unwrap_or_default();
    Ok(sn)
}

pub async fn shared_rooms(user_ids: Vec<OwnedUserId>) -> AppResult<Vec<OwnedRoomId>> {
    let mut user_rooms: Vec<(OwnedUserId, Vec<OwnedRoomId>)> = Vec::new();
    for user_id in user_ids {
        let room_ids = room_users::table
            .filter(room_users::user_id.eq(&user_id))
            .select(room_users::room_id)
            .load::<OwnedRoomId>(&mut connect().await?)
            .await?;
        user_rooms.push((user_id, room_ids));
    }

    let mut shared_rooms = user_rooms.pop().map(|i| i.1).unwrap_or_default();
    if shared_rooms.is_empty() {
        return Ok(shared_rooms);
    }
    while let Some((_user_id, room_ids)) = user_rooms.pop() {
        let set1: HashSet<_> = shared_rooms.into_iter().collect();
        let set2: HashSet<_> = room_ids.into_iter().collect();
        shared_rooms = set1.intersection(&set2).cloned().collect();
        if shared_rooms.is_empty() {
            return Ok(shared_rooms);
        }
    }
    Ok(shared_rooms)
}

pub async fn join_sn(user_id: &UserId, room_id: &RoomId) -> AppResult<Seqnum> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("join"))
        .select(room_users::event_sn)
        .first::<i64>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
pub async fn join_depth(user_id: &UserId, room_id: &RoomId) -> AppResult<u64> {
    let join_sn = join_sn(user_id, room_id).await?;
    events::table
        .filter(events::sn.eq(join_sn))
        .select(events::depth)
        .first::<i64>(&mut connect().await?)
        .await
        .map(|depth| depth as u64)
        .map_err(Into::into)
}
pub async fn join_count(room_id: &RoomId) -> AppResult<i64> {
    let count = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("join"))
        .select(room_users::user_id)
        .count()
        .get_result(&mut connect().await?)
        .await?;
    Ok(count)
}

pub async fn joined_after(user_id: &UserId, room_id: &RoomId, event_depth: u64) -> AppResult<bool> {
    let query = events::table
        .inner_join(event_datas::table.on(event_datas::event_id.eq(events::id)))
        .filter(events::room_id.eq(room_id))
        .filter(events::ty.eq("m.room.member"))
        .filter(events::state_key.eq(user_id.as_str()))
        .filter(events::depth.gt(utils::u64_to_i64(event_depth)))
        .filter(events::is_outlier.eq(false))
        .filter(events::soft_failed.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
            "event_datas.json_data -> 'content' ->> 'membership' = 'join'",
        ));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

pub async fn server_user_joined_after(
    server_name: &ServerName,
    room_id: &RoomId,
    event_depth: u64,
) -> AppResult<bool> {
    let query = events::table
        .inner_join(event_datas::table.on(event_datas::event_id.eq(events::id)))
        .filter(events::room_id.eq(room_id))
        .filter(events::ty.eq("m.room.member"))
        .filter(
            diesel::dsl::sql::<diesel::sql_types::Bool>(
                "substring(events.state_key from position(':' in events.state_key) + 1) = ",
            )
            .bind::<diesel::sql_types::Text, _>(server_name.as_str()),
        )
        .filter(events::depth.gt(utils::u64_to_i64(event_depth)))
        .filter(events::is_outlier.eq(false))
        .filter(events::soft_failed.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(diesel::dsl::sql::<diesel::sql_types::Bool>(
            "event_datas.json_data -> 'content' ->> 'membership' = 'join'",
        ));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

pub async fn knock_sn(user_id: &UserId, room_id: &RoomId) -> AppResult<i64> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("knock"))
        .select(room_users::event_sn)
        .first::<i64>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
pub async fn knock_count(room_id: &RoomId) -> AppResult<i64> {
    let count = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("knock"))
        .select(room_users::user_id)
        .count()
        .get_result(&mut connect().await?)
        .await?;
    Ok(count)
}
pub async fn leave_sn(user_id: &UserId, room_id: &RoomId) -> AppResult<i64> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq("leave"))
        .select(room_users::event_sn)
        .first::<i64>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

#[tracing::instrument]
pub async fn is_invited(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    let query = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq(MembershipState::Invite.to_string()));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

#[tracing::instrument]
pub async fn is_banned(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    let query = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq(MembershipState::Ban.to_string()));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

#[tracing::instrument]
pub async fn is_left(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    let left = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .order_by(room_users::id.desc())
        .select(room_users::membership)
        .first::<String>(&mut connect().await?)
        .await
        .map(|m| m == MembershipState::Leave.to_string())
        .unwrap_or(true);
    Ok(left)
}

#[tracing::instrument]
pub async fn is_knocked(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    let query = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq(MembershipState::Knock.to_string()));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}
#[tracing::instrument]
pub async fn is_joined(user_id: &UserId, room_id: &RoomId) -> AppResult<bool> {
    let joined = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .order_by(room_users::id.desc())
        .select(room_users::membership)
        .first::<String>(&mut connect().await?)
        .await
        .map(|m| m == MembershipState::Join.to_string())
        .unwrap_or(false);
    Ok(joined)
}

#[tracing::instrument]
pub async fn left_sn(room_id: &RoomId, user_id: &UserId) -> AppResult<Seqnum> {
    room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::user_id.eq(user_id))
        .filter(
            room_users::membership
                .eq("leave")
                .or(room_users::membership.eq("ban")),
        )
        .select(room_users::event_sn)
        .first::<Seqnum>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

#[tracing::instrument(level = "trace")]
pub async fn invite_state(
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<Vec<RawJson<AnyStrippedStateEvent>>> {
    if let Some(state) = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq(MembershipState::Invite.to_string()))
        .select(room_users::state_data)
        .first::<Option<JsonValue>>(&mut connect().await?)
        .await
        .unwrap_or_default()
    {
        Ok(serde_json::from_value(state)?)
    } else {
        Ok(vec![])
    }
}

#[tracing::instrument(level = "trace")]
pub async fn membership(user_id: &UserId, room_id: &RoomId) -> AppResult<MembershipState> {
    let membership = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::room_id.eq(room_id))
        .order_by(room_users::id.desc())
        .select(room_users::membership)
        .first::<String>(&mut connect().await?)
        .await;
    if let Ok(membership) = membership {
        Ok(membership.into())
    } else {
        Err(
            MatrixError::not_found(format!("User {user_id} is not a member of room {room_id}"))
                .into(),
        )
    }
}
/// Returns an iterator over all rooms a user left.
#[tracing::instrument]
pub async fn left_rooms(
    user_id: &UserId,
    since_tk: Option<BatchToken>,
) -> AppResult<HashMap<OwnedRoomId, Vec<RawJson<AnySyncStateEvent>>>> {
    let query = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .filter(room_users::membership.eq_any(vec![
            MembershipState::Leave.to_string(),
            MembershipState::Ban.to_string(),
        ]))
        .into_boxed();
    let query = if let Some(since_tk) = since_tk {
        query.filter(room_users::event_sn.ge(since_tk.event_sn()))
    } else {
        query.filter(room_users::forgotten.eq(false))
    };
    let room_event_ids = query
        .select((room_users::room_id, room_users::event_id))
        .load::<(OwnedRoomId, OwnedEventId)>(&mut connect().await?)
        .await
        .map(|rows| {
            let mut map: HashMap<OwnedRoomId, Vec<OwnedEventId>> = HashMap::new();
            for (room_id, event_id) in rows {
                map.entry(room_id).or_default().push(event_id);
            }
            map
        })?;
    let mut room_events = HashMap::new();
    for (room_id, event_ids) in room_event_ids {
        let events = event_datas::table
            .filter(event_datas::event_id.eq_any(&event_ids))
            .select(event_datas::json_data)
            .load::<JsonValue>(&mut connect().await?)
            .await?
            .into_iter()
            .filter_map(|value| RawJson::<AnySyncStateEvent>::from_value(&value).ok())
            .collect::<Vec<_>>();
        room_events.insert(room_id, events);
    }
    Ok(room_events)
}

pub async fn get_tags(user_id: &UserId, room_id: &RoomId) -> AppResult<Vec<DbRoomTag>> {
    let tags = room_tags::table
        .filter(room_tags::user_id.eq(user_id))
        .filter(room_tags::room_id.eq(room_id))
        .load::<DbRoomTag>(&mut connect().await?)
        .await?;
    Ok(tags)
}
pub async fn local_users(room_id: &RoomId) -> AppResult<Vec<OwnedUserId>> {
    let users = room_users::table
        .filter(room_users::room_id.eq(room_id))
        .filter(room_users::membership.eq("join"))
        .select(room_users::user_id)
        .distinct()
        .load::<OwnedUserId>(&mut connect().await?)
        .await?;
    let users = users
        .into_iter()
        .filter(|user_id| user_id.server_name().is_local())
        .collect::<Vec<_>>();
    Ok(users)
}

/// Copies the tags and direct room state from one room to another.
pub async fn copy_room_tags_and_direct_to_room(
    user_id: &UserId,
    old_room_id: &RoomId,
    new_room_id: &RoomId,
) -> AppResult<()> {
    let Ok(mut direct_rooms) = crate::user::get_data::<IndexMap<String, Vec<OwnedRoomId>>>(
        user_id,
        None,
        &GlobalAccountDataEventType::Direct.to_string(),
    )
    .await
    else {
        return Ok(());
    };

    let old_room_id = old_room_id.to_owned();
    for (_key, room_ids) in direct_rooms.iter_mut() {
        if room_ids.contains(&old_room_id) {
            room_ids.retain(|r| r != &old_room_id);
            let new_room_id = new_room_id.to_owned();
            if !room_ids.contains(&new_room_id) {
                room_ids.push(new_room_id);
            }
        }
    }

    crate::user::set_data(
        user_id,
        None,
        &GlobalAccountDataEventType::Direct.to_string(),
        serde_json::to_value(direct_rooms)?,
    )
    .await?;

    let room_tags = get_tags(user_id, &old_room_id).await?;
    for tag in room_tags {
        let DbRoomTag {
            user_id,
            tag,
            content,
            ..
        } = tag;
        let new_tag = NewDbRoomTag {
            user_id,
            room_id: new_room_id.to_owned(),
            tag,
            content,
        };
        diesel::insert_into(room_tags::table)
            .values(&new_tag)
            .execute(&mut connect().await?)
            .await?;
    }
    Ok(())
}

/// Copy all of the push rules from one room to another for a specific user
pub async fn copy_push_rules_from_room_to_room(
    user_id: &UserId,
    _old_room_id: &RoomId,
    new_room_id: &RoomId,
) -> AppResult<()> {
    let Ok(mut user_data_content) = crate::data::user::get_data::<PushRulesEventContent>(
        user_id,
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )
    .await
    else {
        return Ok(());
    };

    let mut new_rules = vec![];
    for push_rule in user_data_content.global.iter() {
        if !push_rule.enabled() {
            continue;
        }

        // Other `AnyPushRuleRef` variants (Override, Content, PostContent, Sender,
        // Underride) are intentionally not copied yet.
        #[allow(clippy::single_match)]
        match push_rule {
            AnyPushRuleRef::Room(rule) => {
                let new_rule = NewPushRule::Room(NewSimplePushRule::new(
                    new_room_id.to_owned(),
                    rule.actions.clone(),
                ));
                new_rules.push(new_rule);
            }
            _ => {}
        }
    }
    for new_rule in new_rules {
        if let Err(e) = user_data_content.global.insert(new_rule, None, None) {
            error!("failed to insert copied push rule: {}", e);
        }
    }

    crate::data::user::set_data(
        user_id,
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(user_data_content)?,
    )
    .await?;
    Ok(())
}
