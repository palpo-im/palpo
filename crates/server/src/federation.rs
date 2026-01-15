use salvo::http::header::AUTHORIZATION;
use salvo::http::headers::authorization::Credentials;

use crate::core::error::{AuthenticateError, ErrorKind};
use crate::core::federation::authentication::XMatrix;
use crate::core::identifiers::*;
use crate::core::room::{AllowRule, JoinRule};
use crate::core::serde::{CanonicalJsonObject, JsonValue};
use crate::core::{MatrixError, signatures};
use crate::{AppError, AppResult, config, room, sending};

mod access_check;
pub mod membership;
pub use access_check::access_check;

#[tracing::instrument(skip(request))]
pub(crate) async fn send_request(
    destination: &ServerName,
    mut request: reqwest::Request,
) -> AppResult<reqwest::Response> {
    if crate::config::get().enabled_federation().is_none() {
        return Err(AppError::public("Federation is disabled."));
    }

    if destination == config::get().server_name {
        return Err(AppError::public(
            "won't send federation request to ourselves",
        ));
    }

    debug!("preparing to send request to {destination}");
    let mut request_map = serde_json::Map::new();

    if let Some(body) = request.body() {
        request_map.insert(
            "content".to_owned(),
            serde_json::from_slice(body.as_bytes().unwrap_or_default())
                .expect("body is valid json, we just created it"),
        );
    };

    request_map.insert("method".to_owned(), request.method().to_string().into());
    request_map.insert(
        "uri".to_owned(),
        format!(
            "{}{}",
            request.url().path(),
            request
                .url()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default()
        )
        .into(),
    );
    request_map.insert(
        "origin".to_owned(),
        config::get().server_name.as_str().into(),
    );
    request_map.insert("destination".to_owned(), destination.as_str().into());

    let mut request_json =
        serde_json::from_value(request_map.into()).expect("valid JSON is valid BTreeMap");

    signatures::sign_json(
        config::get().server_name.as_str(),
        config::keypair(),
        &mut request_json,
    )
    .expect("our request json is what palpo expects");

    let request_json: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&serde_json::to_vec(&request_json).unwrap()).unwrap();

    let signatures = request_json["signatures"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| {
            v.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k, v.as_str().unwrap()))
        });

    for signature_server in signatures {
        for s in signature_server {
            request.headers_mut().insert(
                AUTHORIZATION,
                XMatrix::parse(format!(
                    "X-Matrix origin=\"{}\",destination=\"{}\",key=\"{}\",sig=\"{}\"",
                    config::get().server_name,
                    destination,
                    s.0,
                    s.1
                ))
                .expect("When signs JSON, it produces a valid base64 signature. All other types are valid ServerNames or OwnedKeyId")
                .encode(),
            );
        }
    }

    let url = request.url().clone();

    debug!("sending request to {destination} at {url}");
    let response = sending::federation_client().execute(request).await;

    match response {
        Ok(response) => {
            let status = response.status();
            if status == 200 {
                Ok(response)
            } else {
                let authenticate = if let Some(header) = response.headers().get("WWW-Authenticate")
                {
                    if let Ok(header) = header.to_str() {
                        AuthenticateError::from_str(header)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let body = response.text().await.unwrap_or_default();
                warn!("answer from {destination}({url}) {status}: {body}");
                let mut extra = serde_json::from_str::<serde_json::Map<String, JsonValue>>(&body)
                    .unwrap_or_default();
                let msg = extra
                    .remove("error")
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .unwrap_or("parse remote response data failed".to_owned());
                Err(MatrixError {
                    status_code: Some(status),
                    authenticate,
                    kind: serde_json::from_value(JsonValue::Object(extra))
                        .unwrap_or(ErrorKind::Unknown),
                    body: msg.into(),
                }
                .into())
            }
        }
        Err(e) => {
            warn!(
                "could not send request to {} at {}: {}",
                destination, url, e
            );
            Err(e.into())
        }
    }
}

/// Checks whether the given user can join the given room via a restricted join.
pub(crate) async fn user_can_perform_restricted_join(
    user_id: &UserId,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    join_rule: Option<&JoinRule>,
) -> AppResult<bool> {
    use RoomVersionId::*;

    // restricted rooms are not supported on <=v7
    if matches!(room_version_id, V1 | V2 | V3 | V4 | V5 | V6 | V7) {
        return Ok(false);
    }

    if room::user::is_joined(user_id, room_id).unwrap_or(false) {
        // joining user is already joined, there is nothing we need to do
        return Ok(false);
    }

    if room::user::is_invited(user_id, room_id).unwrap_or(false) {
        return Ok(true);
    }

    let join_rule = match join_rule {
        Some(rule) => rule.to_owned(),
        None => {
            // If no join rule is provided, we need to fetch it from the room state
            let Ok(join_rule) = room::get_join_rule(room_id) else {
                return Ok(false);
            };
            join_rule
        }
    };

    let (JoinRule::Restricted(r) | JoinRule::KnockRestricted(r)) = join_rule else {
        return Ok(false);
    };

    if r.allow.is_empty() {
        tracing::info!("`{room_id}` is restricted but the allow key is empty");
        return Ok(false);
    }

    if r.allow
        .iter()
        .filter_map(|rule| {
            if let AllowRule::RoomMembership(membership) = rule {
                Some(membership)
            } else {
                None
            }
        })
        .any(|m| {
            room::is_server_joined(&config::get().server_name, &m.room_id).unwrap_or(false)
                && room::user::is_joined(user_id, &m.room_id).unwrap_or(false)
        })
    {
        Ok(true)
    } else {
        Err(MatrixError::unable_to_authorize_join(
            "joining user is not known to be in any required room",
        )
        .into())
    }
}

pub(crate) fn maybe_strip_event_id(
    pdu_json: &mut CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> bool {
    match room_version_id {
        RoomVersionId::V1 | RoomVersionId::V2 => false,
        _ => {
            pdu_json.remove("event_id");
            true
        }
    }
}
