//! Admin Event Reports API
//!
//! - GET /_synapse/admin/v1/event_reports
//! - GET /_synapse/admin/v1/event_reports/{report_id}
//! - DELETE /_synapse/admin/v1/event_reports/{report_id}

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::{EmptyResult, JsonResult, MatrixError, data, empty_ok, json_ok};

pub fn router() -> Router {
    Router::new()
        .push(Router::with_path("v1/event_reports").get(list_event_reports))
        .push(
            Router::with_path("v1/event_reports/{report_id}")
                .get(get_event_report)
                .delete(delete_event_report),
        )
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventReport {
    pub id: i64,
    pub received_ts: i64,
    pub room_id: String,
    pub event_id: String,
    pub user_id: String,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
}

impl From<data::room::EventReportInfo> for EventReport {
    fn from(info: data::room::EventReportInfo) -> Self {
        Self {
            id: info.id,
            received_ts: info.received_ts,
            room_id: info.room_id.to_string(),
            event_id: info.event_id.to_string(),
            user_id: info.user_id.to_string(),
            reason: info.reason,
            score: info.score,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventReportsResponse {
    pub event_reports: Vec<EventReport>,
    pub total: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventReportDetailResponse {
    pub id: i64,
    pub received_ts: i64,
    pub room_id: String,
    pub event_id: String,
    pub user_id: String,
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_json: Option<serde_json::Value>,
}

impl From<data::room::DbEventReport> for EventReportDetailResponse {
    fn from(db: data::room::DbEventReport) -> Self {
        Self {
            id: db.id,
            received_ts: db.received_ts,
            room_id: db.room_id.to_string(),
            event_id: db.event_id.to_string(),
            user_id: db.user_id.to_string(),
            reason: db.reason,
            score: db.score,
            event_json: db.content,
        }
    }
}

#[derive(Debug, Deserialize, ToParameters)]
pub struct ListEventReportsQuery {
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub room_id: Option<String>,
}

/// GET /_synapse/admin/v1/event_reports
///
/// List all reported events with pagination and filtering
#[endpoint(operation_id = "list_event_reports")]
pub fn list_event_reports(query: ListEventReportsQuery) -> JsonResult<EventReportsResponse> {
    let from = query.from.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).min(1000);

    let user_id = if let Some(ref uid) = query.user_id {
        Some(
            UserId::parse(uid)
                .map_err(|_| MatrixError::invalid_param("Invalid user_id"))?
                .to_owned(),
        )
    } else {
        None
    };

    let room_id = if let Some(ref rid) = query.room_id {
        Some(
            RoomId::parse(rid)
                .map_err(|_| MatrixError::invalid_param("Invalid room_id"))?
                .to_owned(),
        )
    } else {
        None
    };

    let filter = data::room::EventReportFilter {
        from: Some(from),
        limit: Some(limit),
        direction: query.dir,
        user_id,
        room_id,
    };

    let (reports, total) = data::room::list_event_reports(&filter)?;

    let event_reports: Vec<EventReport> = reports.into_iter().map(Into::into).collect();
    let next_token = if (from + limit) < total {
        Some(from + event_reports.len() as i64)
    } else {
        None
    };

    json_ok(EventReportsResponse {
        event_reports,
        total,
        next_token,
    })
}

/// GET /_synapse/admin/v1/event_reports/{report_id}
///
/// Get details of a specific event report including the event JSON
#[endpoint(operation_id = "get_event_report")]
pub fn get_event_report(report_id: PathParam<i64>) -> JsonResult<EventReportDetailResponse> {
    let report_id = report_id.into_inner();

    let report = data::room::get_event_report(report_id)?
        .ok_or_else(|| MatrixError::not_found(format!("Event report {} not found", report_id)))?;

    json_ok(report.into())
}

/// DELETE /_synapse/admin/v1/event_reports/{report_id}
///
/// Delete an event report
#[endpoint(operation_id = "delete_event_report")]
pub fn delete_event_report(report_id: PathParam<i64>) -> EmptyResult {
    let report_id = report_id.into_inner();

    let deleted = data::room::delete_event_report(report_id)?;
    if !deleted {
        return Err(
            MatrixError::not_found(format!("Event report {} not found", report_id)).into(),
        );
    }

    empty_ok()
}
