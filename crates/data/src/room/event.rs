use diesel::prelude::*;

use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::room::NewDbEventPushAction;
use crate::schema::*;
use crate::{DataResult, connect};

#[tracing::instrument]
pub fn upsert_push_action(action: &NewDbEventPushAction) -> DataResult<()> {
    diesel::insert_into(event_push_actions::table)
        .values(action)
        .on_conflict_do_nothing()
        .execute(&mut connect()?)?;
    Ok(())
}

/// A row from event_push_actions for the notifications endpoint.
#[derive(Queryable, Debug, Clone)]
pub struct PushActionRow {
    pub id: i64,
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub event_sn: i64,
    pub user_id: OwnedUserId,
    pub profile_tag: String,
    pub actions: JsonValue,
    pub topological_ordering: Option<i64>,
    pub stream_ordering: Option<i64>,
    pub notify: bool,
    pub highlight: bool,
    pub unread: bool,
    pub thread_id: Option<OwnedEventId>,
}

/// Get push actions for a user, ordered by stream_ordering descending.
/// Used by the GET /notifications endpoint.
pub fn get_push_actions_for_user(
    user_id: &UserId,
    from: Option<i64>,
    limit: i64,
    only_highlight: bool,
) -> DataResult<Vec<PushActionRow>> {
    let mut query = event_push_actions::table
        .filter(event_push_actions::user_id.eq(user_id))
        .filter(event_push_actions::notify.eq(true))
        .into_boxed();

    if only_highlight {
        query = query.filter(event_push_actions::highlight.eq(true));
    }

    if let Some(from_id) = from {
        query = query.filter(event_push_actions::id.lt(from_id));
    }

    query
        .order_by(event_push_actions::id.desc())
        .limit(limit)
        .load::<PushActionRow>(&mut connect()?)
        .map_err(Into::into)
}

/// Check if the user has a read receipt at or after the given event.
pub fn has_user_read_event(user_id: &UserId, room_id: &RoomId, event_sn: i64) -> bool {
    event_receipts::table
        .filter(event_receipts::user_id.eq(user_id))
        .filter(event_receipts::room_id.eq(room_id))
        .filter(event_receipts::event_sn.ge(event_sn))
        .count()
        .get_result::<i64>(&mut connect().unwrap_or_else(|_| panic!("db connect failed")))
        .unwrap_or(0)
        > 0
}
