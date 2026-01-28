use diesel::prelude::*;

use crate::core::identifiers::*;
use crate::core::UnixMillis;
use crate::room::DbEvent;
use crate::schema::*;
use crate::{DataResult, connect};

/// Get PDUs by room with pagination
pub fn get_pdus_by_room(
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

    query.limit(limit).load(&mut connect()?).map_err(Into::into)
}

/// Get PDU by timestamp
pub fn get_pdu_by_timestamp(
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

    query.first(&mut connect()?).optional().map_err(Into::into)
}
