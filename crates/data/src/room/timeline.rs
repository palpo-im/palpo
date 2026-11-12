use diesel::prelude::*;
use diesel_async::AsyncPgConnection;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::room::DbEvent;
use crate::schema::*;
use crate::{DataResult, connect};

const ROOM_EVENT_APPEND_LOCK_NAMESPACE: i64 = 7_252_664_853;

/// Serialize locally authored events for one room across all server processes.
///
/// The caller must keep the surrounding transaction open until the event has been
/// built and published. A transaction-scoped lock is released automatically on
/// success, error, cancellation, or connection loss.
///
/// Local builders hold this from prev-event/state reads through publication.
/// Live federation promotions, soft-fail tip updates and purge also participate;
/// ordinary remote DAG forks are still resolved using Matrix state resolution.
///
/// The connection's `statement_timeout` also bounds the wait, so a room whose append lock
/// is held for longer fails the request instead of blocking forever.
pub async fn lock_event_append(conn: &mut AsyncPgConnection, room_id: &RoomId) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2))")
        .bind::<diesel::sql_types::Text, _>(room_id.as_str())
        .bind::<diesel::sql_types::BigInt, _>(ROOM_EVENT_APPEND_LOCK_NAMESPACE)
        .execute(conn)
        .await?;
    Ok(())
}

/// Get PDUs by room with pagination
pub async fn get_pdus_by_room(
    room_id: &RoomId,
    from_sn: Option<i64>,
    limit: i64,
    backward: bool,
) -> DataResult<Vec<DbEvent>> {
    let mut query = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::is_outlier.eq(false))
        .into_boxed();

    if let Some(sn) = from_sn {
        if backward {
            query = query.filter(events::sn.lt(sn));
        } else {
            query = query.filter(events::sn.gt(sn));
        }
    }

    if backward {
        query = query.order(events::sn.desc());
    } else {
        query = query.order(events::sn.asc());
    }

    query
        .limit(limit)
        .load(&mut crate::db())
        .await
        .map_err(Into::into)
}

/// Purge room history before a given timestamp.
/// State events (those with state_key set) are preserved.
/// Returns the number of events deleted.
pub async fn purge_room_history(room_id: &RoomId, before_ts: i64) -> DataResult<i64> {
    let before_ts_millis = UnixMillis(u64::try_from(before_ts).map_err(|_| {
        crate::core::MatrixError::invalid_param("purge timestamp must not be negative")
    })?);

    // Each committed batch is a valid stopping point: retrying the same cutoff
    // scans the remaining rows. Neither SQL bindings nor the transaction/Vec grow
    // with the size of a room. Never remove live DAG tips, even on another branch.
    let mut cursor = i64::MIN;
    let mut deleted = 0;
    loop {
        let (count, next_cursor) = crate::coordination_connect()
            .transaction::<_, crate::DataError, _>(async |fence| {
                // Wait on the coordination pool, never consume a query lease
                // while the lock holder needs that same pool to publish.
                lock_event_append(fence, room_id).await?;
                connect()
                    .transaction::<_, crate::DataError, _>(async |conn| {
                        let batch = events::table
                            .filter(events::room_id.eq(room_id))
                            .filter(events::origin_server_ts.lt(before_ts_millis))
                            .filter(events::state_key.is_null())
                            .filter(events::is_outlier.eq(false))
                            .filter(events::sn.gt(cursor))
                            .filter(
                                events::id.ne_all(
                                    event_forward_extremities::table
                                        .select(event_forward_extremities::event_id),
                                ),
                            )
                            .order(events::sn.asc())
                            .limit(500)
                            .select((events::id, events::sn))
                            .load::<(String, i64)>(conn)
                            .await?;
                        let Some((_, next_cursor)) = batch.last() else {
                            return Ok((0, cursor));
                        };
                        let next_cursor = *next_cursor;
                        let event_ids: Vec<_> = batch.iter().map(|(id, _)| id.as_str()).collect();
                        diesel::delete(
                            event_datas::table.filter(event_datas::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_edges::table.filter(event_edges::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_forward_extremities::table
                                .filter(event_forward_extremities::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_backward_extremities::table
                                .filter(event_backward_extremities::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_relations::table.filter(
                                event_relations::event_id
                                    .eq_any(&event_ids)
                                    .or(event_relations::child_id.eq_any(&event_ids)),
                            ),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_receipts::table
                                .filter(event_receipts::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_searches::table
                                .filter(event_searches::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_push_actions::table
                                .filter(event_push_actions::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_points::table.filter(event_points::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(
                            event_missings::table
                                .filter(event_missings::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        diesel::delete(threads::table.filter(threads::event_id.eq_any(&event_ids)))
                            .execute(conn)
                            .await?;
                        diesel::delete(
                            timeline_gaps::table.filter(timeline_gaps::event_id.eq_any(&event_ids)),
                        )
                        .execute(conn)
                        .await?;
                        // Delete the events themselves
                        diesel::delete(events::table.filter(events::id.eq_any(&event_ids)))
                            .execute(conn)
                            .await?;
                        Ok((batch.len() as i64, next_cursor))
                    })
                    .await
            })
            .await?;
        if count == 0 {
            break;
        }
        deleted += count;
        cursor = next_cursor;
        tokio::task::yield_now().await;
    }
    Ok(deleted)
}

/// Get PDU by timestamp
pub async fn get_pdu_by_timestamp(
    room_id: &RoomId,
    ts: i64,
    backward: bool,
) -> DataResult<Option<DbEvent>> {
    let ts_millis = UnixMillis::from_system_time(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(ts as u64),
    )
    .unwrap_or(UnixMillis::now());

    let mut query = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::is_outlier.eq(false))
        .into_boxed();

    if backward {
        query = query
            .filter(events::origin_server_ts.le(ts_millis))
            .order(events::origin_server_ts.desc());
    } else {
        query = query
            .filter(events::origin_server_ts.ge(ts_millis))
            .order(events::origin_server_ts.asc());
    }

    query
        .first(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}
