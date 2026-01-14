//! Admin Scheduled Tasks API
//!
//! - GET /_synapse/admin/v1/scheduled_tasks

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::{JsonResult, MatrixError};

pub fn router() -> Router {
    Router::new().push(Router::with_path("v1/scheduled_tasks").get(list_scheduled_tasks))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduledTasksResponse {
    pub scheduled_tasks: Vec<ScheduledTask>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduledTask {
    pub id: String,
    pub action: String,
    pub status: String,
    pub timestamp_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// GET /_synapse/admin/v1/scheduled_tasks
///
/// List scheduled tasks
#[endpoint]
pub fn list_scheduled_tasks(
    _action_name: QueryParam<String, false>,
    _resource_id: QueryParam<String, false>,
    _job_status: QueryParam<String, false>,
    _max_timestamp: QueryParam<i64, false>,
) -> JsonResult<ScheduledTasksResponse> {
    Err(MatrixError::bad_status(
        Some(salvo::http::StatusCode::NOT_IMPLEMENTED),
        "Scheduled tasks admin API is not enabled on this server",
    )
    .into())
}
