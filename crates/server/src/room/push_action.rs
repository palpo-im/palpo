use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use diesel_async::AsyncPgConnection;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::core::Seqnum;
use crate::core::events::receipt::ReceiptThread;
use crate::core::identifiers::*;
use crate::data::room::{DbReceipt, NewDbEventPushAction};
use crate::data::schema::*;
use crate::{AppError, AppResult};

/// Action insertion, receipt deletion and summary rebuilding share this
/// transaction lock for one user/room, including across server processes.
async fn lock_summary(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<()> {
    let key = serde_json::to_string(&(user_id, room_id))?;
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 7151323))")
        .bind::<Text, _>(key)
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(QueryableByName)]
struct AlreadyRead {
    #[diesel(sql_type = Bool)]
    seen: bool,
}

#[tracing::instrument]
pub async fn upsert_push_action(
    room_id: &RoomId,
    event_id: &EventId,
    user_id: &UserId,
    notify: bool,
    highlight: bool,
) -> AppResult<()> {
    let (event_sn, thread_id) = event_points::table
        .find(event_id)
        .select((event_points::event_sn, event_points::thread_id))
        .first::<(Seqnum, Option<OwnedEventId>)>(&mut crate::data::db())
        .await?;
    let (topological_ordering, stream_ordering) = events::table
        .find(event_id)
        .select((events::topological_ordering, events::stream_ordering))
        .first::<(i64, i64)>(&mut crate::data::db())
        .await?;
    let action = NewDbEventPushAction {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        event_sn,
        user_id: user_id.to_owned(),
        profile_tag: String::new(),
        actions: serde_json::json!([]),
        topological_ordering,
        stream_ordering,
        notify,
        highlight,
        unread: false,
        thread_id,
    };
    crate::data::db()
        .transaction::<_, AppError, _>(async |conn| {
            lock_summary(conn, user_id, room_id).await?;
            // A retried append must not resurrect an action already removed by
            // a receipt. Unthreaded covers all scopes; main and each thread
            // cover only their own timeline.
            let scope = action.thread_id.as_deref().map_or("main", EventId::as_str);
            let read = diesel::sql_query(
                "SELECT EXISTS (SELECT 1 FROM event_receipts \
                 WHERE user_id = $1 AND room_id = $2 AND event_sn >= $3 \
                 AND ty IN ('m.read', 'm.read.private') \
                 AND COALESCE(json_data->>'thread_id', '') IN ('', $4)) AS seen",
            )
            .bind::<Text, _>(user_id.as_str())
            .bind::<Text, _>(room_id.as_str())
            .bind::<BigInt, _>(event_sn)
            .bind::<Text, _>(scope)
            .get_result::<AlreadyRead>(conn)
            .await?;
            if read.seen {
                return Ok(());
            }
            let inserted = diesel::insert_into(event_push_actions::table)
                .values(&action)
                .on_conflict_do_nothing()
                .returning(event_push_actions::id)
                .get_result::<i64>(conn)
                .await
                .optional()?;
            if inserted.is_none() || (!notify && !highlight) {
                return Ok(());
            }
            let conflict = if action.thread_id.is_some() {
                "(user_id, room_id, thread_id) WHERE thread_id IS NOT NULL"
            } else {
                "(user_id, room_id) WHERE thread_id IS NULL"
            };
            diesel::sql_query(format!(
                "INSERT INTO event_push_summaries \
                 (user_id, room_id, thread_id, notification_count, highlight_count, unread_count, stream_ordering) \
                 VALUES ($1, $2, $3, $4, $5, 0, $6) \
                 ON CONFLICT {conflict} DO UPDATE SET \
                 notification_count = event_push_summaries.notification_count + EXCLUDED.notification_count, \
                 highlight_count = event_push_summaries.highlight_count + EXCLUDED.highlight_count, \
                 stream_ordering = GREATEST(event_push_summaries.stream_ordering, EXCLUDED.stream_ordering)"
            ))
            .bind::<Text, _>(user_id.as_str())
            .bind::<Text, _>(room_id.as_str())
            .bind::<Nullable<Text>, _>(action.thread_id.as_deref().map(EventId::as_str))
            .bind::<BigInt, _>(i64::from(notify))
            .bind::<BigInt, _>(i64::from(highlight))
            .bind::<BigInt, _>(stream_ordering)
            .execute(conn)
            .await?;
            Ok(())
        })
        .await
}

/// Clear exactly the receipt's scope and rebuild its summaries in the same
/// transaction. Main and unthreaded are deliberately distinct.
pub async fn remove_actions_until(
    user_id: &UserId,
    room_id: &RoomId,
    event_sn: Seqnum,
    scope: &ReceiptThread,
) -> AppResult<()> {
    crate::data::db()
        .transaction::<_, AppError, _>(async |conn| {
            lock_summary(conn, user_id, room_id).await?;
            clear_locked(conn, user_id, room_id, event_sn, scope).await
        })
        .await
}

/// Client receipt publication and its notification effects have one commit
/// point. A failed rebuild cannot advance a receipt while leaving stale counts.
pub async fn record_read_and_clear(receipt: &DbReceipt, scope: &ReceiptThread) -> AppResult<()> {
    crate::data::db()
        .transaction::<_, AppError, _>(async |conn| {
            lock_summary(conn, &receipt.user_id, &receipt.room_id).await?;
            crate::data::room::receipt::publish_receipt_with_conn(conn, receipt).await?;
            clear_locked(
                conn,
                &receipt.user_id,
                &receipt.room_id,
                receipt.event_sn,
                scope,
            )
            .await
        })
        .await
}

async fn clear_locked(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: &RoomId,
    event_sn: Seqnum,
    scope: &ReceiptThread,
) -> AppResult<()> {
    let target = match scope {
        ReceiptThread::Main => "main",
        ReceiptThread::Thread(id) => id.as_str(),
        ReceiptThread::Unthreaded => "",
        _ => return Err(crate::MatrixError::invalid_param("Unknown receipt scope").into()),
    };
    diesel::sql_query(
        "DELETE FROM event_push_actions WHERE user_id = $1 AND room_id = $2 \
         AND event_sn <= $3 AND \
         ($4 = '' OR ($4 = 'main' AND thread_id IS NULL) OR thread_id = $4)",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(room_id.as_str())
    .bind::<BigInt, _>(event_sn)
    .bind::<Text, _>(target)
    .execute(conn)
    .await?;
    refresh_locked(conn, user_id, room_id).await
}

pub async fn remove_actions_for_room(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    crate::data::db()
        .transaction::<_, AppError, _>(async |conn| {
            lock_summary(conn, user_id, room_id).await?;
            diesel::delete(
                event_push_actions::table
                    .filter(event_push_actions::user_id.eq(user_id))
                    .filter(event_push_actions::room_id.eq(room_id)),
            )
            .execute(conn)
            .await?;
            refresh_locked(conn, user_id, room_id).await
        })
        .await
}

pub async fn refresh_notify_summary(user_id: &UserId, room_id: &RoomId) -> AppResult<()> {
    crate::data::db()
        .transaction::<_, AppError, _>(async |conn| {
            lock_summary(conn, user_id, room_id).await?;
            refresh_locked(conn, user_id, room_id).await
        })
        .await
}

async fn refresh_locked(
    conn: &mut AsyncPgConnection,
    user_id: &UserId,
    room_id: &RoomId,
) -> AppResult<()> {
    diesel::delete(
        event_push_summaries::table
            .filter(event_push_summaries::user_id.eq(user_id))
            .filter(event_push_summaries::room_id.eq(room_id)),
    )
    .execute(conn)
    .await?;
    diesel::sql_query(
        "INSERT INTO event_push_summaries \
         (user_id, room_id, thread_id, notification_count, highlight_count, unread_count, stream_ordering) \
         SELECT $1, $2, thread_id, COUNT(*) FILTER (WHERE notify), \
         COUNT(*) FILTER (WHERE highlight), COUNT(*) FILTER (WHERE unread), \
         COALESCE(MAX(stream_ordering), 1) FROM event_push_actions \
         WHERE user_id = $1 AND room_id = $2 GROUP BY thread_id"
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(room_id.as_str())
    .execute(conn).await?;
    // Preserve an explicit transition back to zero in the main timeline.
    diesel::sql_query(
        "INSERT INTO event_push_summaries \
         (user_id, room_id, notification_count, highlight_count, unread_count, stream_ordering) \
         VALUES ($1, $2, 0, 0, 0, 1) ON CONFLICT (user_id, room_id) \
         WHERE thread_id IS NULL DO NOTHING",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(room_id.as_str())
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
