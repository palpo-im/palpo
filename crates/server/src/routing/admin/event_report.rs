//! Admin Event Reports API
//!
//! - GET /_synapse/admin/v1/event_reports
//! - GET /_synapse/admin/v1/event_reports/{report_id}
//! - DELETE /_synapse/admin/v1/event_reports/{report_id}

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::{JsonResult, MatrixError};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_json: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EmptyResponse {}

/// GET /_synapse/admin/v1/event_reports
///
/// List all reported events
#[endpoint]
pub fn list_event_reports(
    _from: QueryParam<i64, false>,
    _limit: QueryParam<i64, false>,
    _dir: QueryParam<String, false>,
    _user_id: QueryParam<String, false>,
    _room_id: QueryParam<String, false>,
    _event_sender_user_id: QueryParam<String, false>,
) -> JsonResult<EventReportsResponse> {
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Event reports are not enabled on this server",
    )
    .into())
}

/// GET /_synapse/admin/v1/event_reports/{report_id}
///
/// Get details of a specific event report
#[endpoint]
pub fn get_event_report(report_id: PathParam<i64>) -> JsonResult<EventReportDetailResponse> {
    let _report_id = report_id.into_inner();
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Event reports are not enabled on this server",
    )
    .into())
}

/// DELETE /_synapse/admin/v1/event_reports/{report_id}
///
/// Delete an event report
#[endpoint]
pub fn delete_event_report(report_id: PathParam<i64>) -> JsonResult<EmptyResponse> {
    let _report_id = report_id.into_inner();
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Event reports are not enabled on this server",
    )
    .into())
}
