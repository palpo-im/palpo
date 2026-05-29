use std::{fmt::Debug, mem};

use bytes::BytesMut;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use palpo_core::push::PusherIds;
use tracing::{info, warn};

use crate::core::client::push::PusherPostData;
use crate::core::client::push::pusher::PusherAction;
use crate::core::identifiers::*;
use crate::core::{
    MatrixVersion, SendAccessToken,
    client::push::{Device, Notification, NotificationCounts, NotificationPriority},
    events::{AnySyncTimelineEvent, StateEventType, TimelineEventType},
    push::{self, Pusher, PusherKind},
    push::{Action, PushConditionRoomCtx, PushFormat, Ruleset, Tweak},
    serde::RawJson,
};
use crate::pdu::PduEvent;
use crate::schema::*;
use crate::{BAD_QUERY_RATE_LIMITER, DataError, DataResult, JsonValue, MatrixError, db};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = push_rules)]
pub struct DbPushRule {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub rule_id: String,
    pub priority_class: i32,
    pub priority: i32,
    pub conditions: JsonValue,
    pub actions: JsonValue,
    pub enabled: bool,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = push_rules)]
pub struct NewDbPushRule {
    pub user_id: OwnedUserId,
    pub rule_id: String,
    pub priority_class: i32,
    pub priority: i32,
    pub conditions: JsonValue,
    pub actions: JsonValue,
    pub enabled: bool,
}
// impl TryInto<PushRule> for DbPushRule {
//     type Error = DataError;
//     fn try_into(self) -> DataResult<PushRule> {
//         let Self {
//             user_id,
//             rule_id,
//             priority_class,
//             priority,
//             conditions,
//             actions,
//             enabled,
//             ..
//         } = self;
//         Ok(Pusher {
//             ids: PusherIds { app_id, pushkey },
//             profile_tag,
//             kind: PusherKind::try_new(&kind, data)?,
//             app_display_name,
//             device_display_name,
//             lang,
//         })
//     }
// }

pub async fn get_push_rules(user_id: &UserId) -> DataResult<Vec<DbPushRule>> {
    let push_rules = push_rules::table
        .filter(push_rules::user_id.eq(user_id))
        .order_by((push_rules::priority_class.asc(), push_rules::priority.asc()))
        .load::<DbPushRule>(&mut db::connect().await?)
        .await?;
    Ok(push_rules)
}
