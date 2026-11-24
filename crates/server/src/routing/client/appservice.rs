use salvo::oapi::extract::{JsonBody, PathParam};
use salvo::prelude::*;
use serde_json::Map;

use crate::core::OwnedTransactionId;
use crate::core::appservice::ping::{SendPingReqBody, SendPingResBody, send_ping_request};
use crate::{AppError, AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

pub fn authed_router() -> Router {
    Router::with_path("appservice/{appservice_id}/ping").post(ping)
}

#[endpoint]
async fn ping(
    _aa: AuthArgs,
    appservice_id: PathParam<OwnedTransactionId>,
    body: JsonBody<SendPingReqBody>,
    depot: &mut Depot,
) -> JsonResult<SendPingResBody> {
    let appservice_id = appservice_id.into_inner();
    let authed = depot.authed_info()?;
    let Some(appservice) = authed.appservice.as_ref() else {
        return Err(MatrixError::forbidden(
            "This endpoint can only be called by appservices.",
            None,
        )
        .into());
    };
    let registration = &appservice.registration;

    if appservice_id != registration.id {
        return Err(MatrixError::forbidden(
            "Appservices can only ping themselves (wrong appservice ID).",
            None,
        )
        .into());
    }

    let Some(url) = registration
        .url
        .as_deref()
        .filter(|url| !url.is_empty() && *url != "null")
    else {
        return Err(MatrixError::url_not_set(
            "Appservice does not have a URL set, there is nothing to ping.",
        )
        .into());
    };

    let timer = tokio::time::Instant::now();
    let mut request = send_ping_request(url, body.into_inner())?.into_inner();
    let mut authorization =
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", registration.hs_token))
            .map_err(|_| AppError::public("Invalid application-service credential"))?;
    authorization.set_sensitive(true);
    request
        .headers_mut()
        .insert(reqwest::header::AUTHORIZATION, authorization);
    let status = crate::appservice::send_request(registration.clone(), request)
        .await?
        .status();
    if status != StatusCode::OK {
        let mut error = MatrixError::bad_status(
            Some(status),
            Map::from_iter([
                (
                    "error".to_owned(),
                    "Appservice returned a bad ping status.".into(),
                ),
                ("status".to_owned(), status.as_u16().into()),
            ]),
        );
        error.status_code = Some(StatusCode::BAD_GATEWAY);
        return Err(error.into());
    }

    json_ok(SendPingResBody::new(timer.elapsed()))
}

#[cfg(test)]
mod tests;
