use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::room::DbEvent;
use crate::schema::*;
use crate::{DataResult, connect};

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
        .load(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Purge room history before a given timestamp.
/// State events (those with state_key set) are preserved.
/// Returns the number of events deleted.
pub async fn purge_room_history(room_id: &RoomId, before_ts: i64) -> DataResult<i64> {
    let before_ts_millis = UnixMillis::from_system_time(
        std::time::UNIX_EPOCH + std::time::Duration::from_millis(before_ts as u64),
    )
    .unwrap_or(UnixMillis::now());

    let mut conn = connect().await?;

    // Collect event IDs and SNs to purge (skip state events)
    let to_purge: Vec<(String, i64)> = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::origin_server_ts.lt(before_ts_millis))
        .filter(events::state_key.is_null())
        .select((events::id, events::sn))
        .load::<(String, i64)>(&mut conn)
        .await?;

    if to_purge.is_empty() {
        return Ok(0);
    }

    let event_ids: Vec<&str> = to_purge.iter().map(|(id, _)| id.as_str()).collect();

    // Run all cascading deletes inside a single transaction so we cannot leave
    // dangling rows in event_datas/event_edges/etc. if one of the statements
    // fails mid-purge (transient db error, statement timeout, ...). Without
    // the transaction, a half-finished purge would leak event-relations
    // pointing at events that no longer exist, which then resurface as
    // hard-to-debug consistency errors on subsequent timeline reads.
    conn.transaction::<_, DieselError, _>(async |conn| {
        // Delete from related tables (no foreign key constraints, order doesn't matter)
        for chunk in event_ids.chunks(500) {
            diesel::delete(event_datas::table.filter(event_datas::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(event_edges::table.filter(event_edges::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(
                event_forward_extremities::table
                    .filter(event_forward_extremities::event_id.eq_any(chunk)),
            )
            .execute(conn)
            .await?;
            diesel::delete(
                event_backward_extremities::table
                    .filter(event_backward_extremities::event_id.eq_any(chunk)),
            )
            .execute(conn)
            .await?;
            diesel::delete(event_relations::table.filter(event_relations::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(event_receipts::table.filter(event_receipts::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(event_searches::table.filter(event_searches::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(
                event_push_actions::table.filter(event_push_actions::event_id.eq_any(chunk)),
            )
            .execute(conn)
            .await?;
            diesel::delete(event_points::table.filter(event_points::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(event_missings::table.filter(event_missings::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(threads::table.filter(threads::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            diesel::delete(timeline_gaps::table.filter(timeline_gaps::event_id.eq_any(chunk)))
                .execute(conn)
                .await?;
            // Delete the events themselves
            diesel::delete(events::table.filter(events::id.eq_any(chunk)))
                .execute(conn)
                .await?;
        }
        Ok(())
    })
    .await?;

    Ok(to_purge.len() as i64)
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
        .first(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}
