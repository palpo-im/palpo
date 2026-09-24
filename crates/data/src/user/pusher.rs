use std::fmt::Debug;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use palpo_core::push::PusherIds;

use crate::core::UnixMillis;
use crate::core::events::AnySyncTimelineEvent;
use crate::core::events::room::power_levels::RoomPowerLevels;
use crate::core::identifiers::*;
use crate::core::push::{
    Action, PushConditionPowerLevelsCtx, PushConditionRoomCtx, Pusher, PusherKind, Ruleset,
};
use crate::core::serde::{JsonValue, RawJson};
use crate::schema::*;
use crate::{DataError, DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_pushers)]
pub struct DbPusher {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub kind: String,
    pub app_id: String,
    pub app_display_name: String,
    pub device_id: OwnedDeviceId,
    pub device_display_name: String,
    pub access_token_id: Option<i64>,
    pub profile_tag: Option<String>,
    pub pushkey: String,
    pub lang: String,
    pub data: JsonValue,
    pub enabled: bool,
    pub last_stream_ordering: Option<i64>,
    pub last_success: Option<i64>,
    pub failing_since: Option<i64>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_pushers)]
pub struct NewDbPusher {
    pub user_id: OwnedUserId,
    pub kind: String,
    pub app_id: String,
    pub app_display_name: String,
    pub device_id: OwnedDeviceId,
    pub device_display_name: String,
    pub access_token_id: Option<i64>,
    pub profile_tag: Option<String>,
    pub pushkey: String,
    pub lang: String,
    pub data: JsonValue,
    pub enabled: bool,
    pub created_at: UnixMillis,
}
impl TryInto<Pusher> for DbPusher {
    type Error = DataError;
    fn try_into(self) -> DataResult<Pusher> {
        let Self {
            profile_tag,
            kind,
            app_id,
            app_display_name,
            device_display_name,
            pushkey,
            lang,
            data,
            ..
        } = self;
        Ok(Pusher {
            ids: PusherIds { app_id, pushkey },
            profile_tag,
            kind: PusherKind::try_new(&kind, data)?,
            app_display_name,
            device_display_name,
            lang,
        })
    }
}

pub async fn get_pusher(user_id: &UserId, pushkey: &str) -> DataResult<Option<Pusher>> {
    let pusher = user_pushers::table
        .filter(user_pushers::user_id.eq(user_id))
        .filter(user_pushers::pushkey.eq(pushkey))
        .order_by(user_pushers::id.desc())
        .first::<DbPusher>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(pusher) = pusher {
        pusher.try_into().map(Option::Some)
    } else {
        Ok(None)
    }
}

pub async fn get_pushers(user_id: &UserId) -> DataResult<Vec<DbPusher>> {
    user_pushers::table
        .filter(user_pushers::user_id.eq(user_id))
        .order_by(user_pushers::id.desc())
        .load::<DbPusher>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Evaluates `ruleset` for `pdu` on behalf of `user`.
///
/// `member_count` is the number of joined members in the room, which the
/// `room_member_count` condition is matched against (the default
/// `.m.rule.room_one_to_one` rules, for instance, match when it is 2).
pub async fn get_actions<'a>(
    user: &UserId,
    ruleset: &'a Ruleset,
    power_levels: &RoomPowerLevels,
    pdu: &RawJson<AnySyncTimelineEvent>,
    room_id: &RoomId,
    member_count: u64,
) -> DataResult<&'a [Action]> {
    let power_levels = PushConditionPowerLevelsCtx {
        users: power_levels.users.clone(),
        users_default: power_levels.users_default,
        notifications: power_levels.notifications.clone(),
        rules: power_levels.rules.clone(),
    };
    let ctx = PushConditionRoomCtx {
        room_id: room_id.to_owned(),
        member_count,
        user_id: user.to_owned(),
        user_display_name: crate::user::display_name(user)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| user.localpart().to_owned()),
        power_levels: Some(power_levels),
        // #[cfg(feature = "unstable-msc3931")]
        supported_features: vec![],
        // #[cfg(feature = "unstable-msc4306")]
        has_thread_subscription_fn: None,
    };

    Ok(ruleset.get_actions(pdu, &ctx).await)
}

pub async fn get_push_keys(user_id: &UserId) -> DataResult<Vec<String>> {
    user_pushers::table
        .filter(user_pushers::user_id.eq(user_id))
        .select(user_pushers::pushkey)
        .load::<String>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn delete_user_pushers(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_pushers::table.filter(user_pushers::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn delete_device_pushers(user_id: &UserId, device_id: &DeviceId) -> DataResult<()> {
    diesel::delete(
        user_pushers::table
            .filter(user_pushers::user_id.eq(user_id))
            .filter(user_pushers::device_id.eq(device_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Remove the pusher identified by `(user_id, app_id, pushkey)`.
pub async fn delete_pusher(user_id: &UserId, app_id: &str, pushkey: &str) -> DataResult<()> {
    diesel::delete(
        user_pushers::table
            .filter(user_pushers::user_id.eq(user_id))
            .filter(user_pushers::pushkey.eq(pushkey))
            .filter(user_pushers::app_id.eq(app_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Create or replace the pusher identified by `(user_id, app_id, pushkey)`.
///
/// A user has at most one pusher per `(app_id, pushkey)`, so an existing one is replaced.
/// Unless `append` is set, pushers with the same `(app_id, pushkey)` belonging to other users
/// are removed as well, as `POST /pushers/set` requires: a device that is now signed in to
/// another account must stop receiving notifications for the previous one.
pub async fn set_pusher(new_pusher: &NewDbPusher, append: bool) -> DataResult<()> {
    let mut conn = connect().await?;
    conn.transaction::<_, DataError, _>(async |conn| {
        let same_key = user_pushers::table
            .filter(user_pushers::app_id.eq(&new_pusher.app_id))
            .filter(user_pushers::pushkey.eq(&new_pusher.pushkey));
        if append {
            diesel::delete(same_key.filter(user_pushers::user_id.eq(&new_pusher.user_id)))
                .execute(conn)
                .await?;
        } else {
            diesel::delete(same_key).execute(conn).await?;
        }
        diesel::insert_into(user_pushers::table)
            .values(new_pusher)
            .execute(conn)
            .await?;
        Ok(())
    })
    .await
}
