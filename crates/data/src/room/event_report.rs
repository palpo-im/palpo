use diesel::prelude::*;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::schema::*;
use crate::{DataResult, connect};

/// Database model for event reports
#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = event_reports)]
pub struct DbEventReport {
    pub id: i64,
    pub received_ts: i64,
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub user_id: OwnedUserId,
    pub reason: Option<String>,
    pub content: Option<JsonValue>,
    pub score: Option<i64>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = event_reports)]
pub struct NewDbEventReport {
    pub received_ts: i64,
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub user_id: OwnedUserId,
    pub reason: Option<String>,
    pub content: Option<JsonValue>,
    pub score: Option<i64>,
}

impl NewDbEventReport {
    pub fn new(
        room_id: OwnedRoomId,
        event_id: OwnedEventId,
        user_id: OwnedUserId,
        reason: Option<String>,
        content: Option<JsonValue>,
        score: Option<i64>,
    ) -> Self {
        Self {
            received_ts: UnixMillis::now().get() as i64,
            room_id,
            event_id,
            user_id,
            reason,
            content,
            score,
        }
    }
}

/// Info returned by the admin API
#[derive(Debug, Clone)]
pub struct EventReportInfo {
    pub id: i64,
    pub received_ts: i64,
    pub room_id: OwnedRoomId,
    pub event_id: OwnedEventId,
    pub user_id: OwnedUserId,
    pub reason: Option<String>,
    pub score: Option<i64>,
}

impl From<DbEventReport> for EventReportInfo {
    fn from(db: DbEventReport) -> Self {
        Self {
            id: db.id,
            received_ts: db.received_ts,
            room_id: db.room_id,
            event_id: db.event_id,
            user_id: db.user_id,
            reason: db.reason,
            score: db.score,
        }
    }
}

/// Filter options for listing event reports
#[derive(Debug, Clone, Default)]
pub struct EventReportFilter {
    pub from: Option<i64>,
    pub limit: Option<i64>,
    pub direction: Option<String>,
    pub user_id: Option<OwnedUserId>,
    pub room_id: Option<OwnedRoomId>,
}

/// Create a new event report
pub fn create_event_report(report: NewDbEventReport) -> DataResult<i64> {
    let result = diesel::insert_into(event_reports::table)
        .values(&report)
        .returning(event_reports::id)
        .get_result::<i64>(&mut connect()?)?;
    Ok(result)
}

/// List event reports with pagination and filtering
pub fn list_event_reports(filter: &EventReportFilter) -> DataResult<(Vec<EventReportInfo>, i64)> {
    let mut count_query = event_reports::table.into_boxed();
    let mut query = event_reports::table.into_boxed();

    // Apply user_id filter
    if let Some(ref user_id) = filter.user_id {
        count_query = count_query.filter(event_reports::user_id.eq(user_id));
        query = query.filter(event_reports::user_id.eq(user_id));
    }

    // Apply room_id filter
    if let Some(ref room_id) = filter.room_id {
        count_query = count_query.filter(event_reports::room_id.eq(room_id));
        query = query.filter(event_reports::room_id.eq(room_id));
    }

    // Get total count
    let total = count_query.count().get_result::<i64>(&mut connect()?)?;

    // Apply ordering - default is backwards (newest first)
    let direction_forward = filter
        .direction
        .as_ref()
        .map(|d| d == "f")
        .unwrap_or(false);

    if direction_forward {
        query = query.order(event_reports::id.asc());
    } else {
        query = query.order(event_reports::id.desc());
    }

    // Apply pagination
    if let Some(from) = filter.from {
        query = query.offset(from);
    }

    let limit = filter.limit.unwrap_or(100).min(1000);
    query = query.limit(limit);

    let reports = query.load::<DbEventReport>(&mut connect()?)?;

    Ok((reports.into_iter().map(Into::into).collect(), total))
}

/// Get a single event report by ID
pub fn get_event_report(report_id: i64) -> DataResult<Option<DbEventReport>> {
    event_reports::table
        .find(report_id)
        .first::<DbEventReport>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

/// Delete an event report by ID
/// Returns true if deleted, false if not found
pub fn delete_event_report(report_id: i64) -> DataResult<bool> {
    let result = diesel::delete(event_reports::table.find(report_id)).execute(&mut connect()?)?;
    Ok(result > 0)
}

/// Get event reports count for statistics
pub fn count_event_reports() -> DataResult<i64> {
    event_reports::table
        .count()
        .get_result::<i64>(&mut connect()?)
        .map_err(Into::into)
}
