//! Admin Event API
//!
//! - GET /_synapse/admin/v1/fetch_event/{event_id}

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::core::identifiers::*;
use crate::room::timeline;
use crate::{JsonResult, MatrixError, json_ok};

pub fn router() -> Router {
    Router::new().push(Router::with_path("v1/fetch_event/{event_id}").get(fetch_event))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FetchEventResponse {
    pub event: serde_json::Value,
}

/// GET /_synapse/admin/v1/fetch_event/{event_id}
///
/// Fetch a single event by ID
#[endpoint]
pub fn fetch_event(event_id: PathParam<OwnedEventId>) -> JsonResult<FetchEventResponse> {
    let event_id = event_id.into_inner();

    let pdu = timeline::get_pdu(&event_id).map_err(|_| MatrixError::not_found("Event not found"))?;

    let event = serde_json::to_value(&pdu).unwrap_or_default();

    json_ok(FetchEventResponse { event })
}
