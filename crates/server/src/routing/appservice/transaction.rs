use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::appservice::event::PushEventsReqBody;
use crate::core::identifiers::*;
use crate::{AuthArgs, EmptyResult, MatrixError, empty_ok};

pub fn router() -> Router {
    Router::with_path("transactions/{txn_id}").put(send_event)
}

/// # `PUT /_matrix/app/v1/transactions/{txnId}`
///
/// This API is called by the homeserver when it wants to push an event
/// (or batch of events) to the application service.
///
/// Note: The application service should respond with a `200 OK` status code.
/// The homeserver will retry the request if it does not receive a `200 OK`.
#[endpoint]
async fn send_event(
    aa: AuthArgs,
    txn_id: PathParam<String>,
    body: JsonBody<PushEventsReqBody>,
) -> EmptyResult {
    let token = aa.require_access_token()?;

    // Validate the hs_token: the calling homeserver must authenticate with the
    // hs_token that matches one of our registered appservices.
    let appservices = crate::appservices();
    let appservice = appservices
        .iter()
        .find(|a| a.hs_token == token)
        .ok_or_else(|| MatrixError::forbidden("Invalid hs_token", None))?;

    let txn_id = txn_id.into_inner();
    let body = body.into_inner();

    debug!(
        "Received appservice transaction {txn_id} for '{}' with {} events",
        appservice.id,
        body.events.len(),
    );

    for event in &body.events {
        trace!("Appservice transaction event: {:?}", event);
    }

    empty_ok()
}
