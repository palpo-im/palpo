use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::policy::sign_event::{PolicySignEventReqBody, PolicySignEventResBody};
use crate::core::identifiers::*;
use crate::core::room_version_rules::{EventIdFormatVersion, RoomVersionRules};
use crate::core::serde::CanonicalJsonObject;
use crate::{AppError, AppResult, AuthArgs, JsonResult, MatrixError, config, hoops, json_ok};

pub fn router() -> Router {
    Router::with_path("policy")
        .hoop(check_policy_server_enabled)
        .hoop(hoops::auth_by_signatures)
        .oapi_tag("policy")
        .push(Router::with_path("v1/sign").post(sign_event))
}

#[handler]
async fn check_policy_server_enabled() -> AppResult<()> {
    if config::get().enabled_federation().is_none() {
        Err(AppError::public("Federation is disabled."))
    } else {
        Ok(())
    }
}

#[endpoint]
async fn sign_event(
    _aa: AuthArgs,
    body: JsonBody<PolicySignEventReqBody>,
) -> JsonResult<PolicySignEventResBody> {
    let object: CanonicalJsonObject = serde_json::from_str(body.0.0.get()).map_err(|_| {
        MatrixError::bad_json("Policy Server signing request must be a JSON object")
    })?;

    let room_id = object
        .get("room_id")
        .and_then(|value| value.as_str())
        .and_then(|value| RoomId::parse(value).ok())
        .ok_or_else(|| MatrixError::bad_json("Policy Server signing request has no room_id"))?;

    // MSC4284 has a Policy Server answer 404 for rooms it does not serve. We only serve
    // rooms we are in, which is also what tells us the room version to redact against.
    let rules = match crate::room::get_version(&room_id).await {
        Ok(room_version) => crate::room::get_version_rules(&room_version)?,
        Err(_) => return Err(MatrixError::not_found("Unknown room").into()),
    };

    let signature = sign_policy_event(object, &rules)?;

    json_ok(PolicySignEventResBody::new(
        config::get().server_name.clone(),
        signature,
    ))
}

/// Signs the event the same way a receiver verifies it: over the redacted PDU, with
/// `signatures` and `unsigned` removed.
///
/// Signing the unredacted event instead produces a signature that every implementation --
/// including palpo's own `verify_policy_server_signature` -- rejects for any event whose
/// content the redaction algorithm strips.
fn sign_policy_event(
    mut object: CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<String> {
    if object.get("type").and_then(|value| value.as_str()) == Some("m.room.policy")
        && object.get("state_key").and_then(|value| value.as_str()) == Some("")
    {
        return Err(MatrixError::forbidden(
            "Policy Server configuration events must not be signed by this endpoint",
            None,
        )
        .into());
    }

    // `event_id` is only part of the PDU in room version 1; senders that keep a copy of it
    // alongside the event must not have it counted towards the signature in later versions.
    if rules.event_id_format != EventIdFormatVersion::V1 {
        object.remove("event_id");
    }

    crate::room::policy::sign_locally(&object, rules)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::sign_policy_event;
    use crate::core::room_version_rules::RoomVersionRules;
    use crate::core::serde::CanonicalJsonObject;

    fn object(value: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn policy_config_event_is_rejected() {
        let pdu = object(json!({
            "type": "m.room.policy",
            "state_key": "",
            "room_id": "!room:example.org",
            "content": {}
        }));

        assert!(sign_policy_event(pdu, &RoomVersionRules::V11).is_err());
    }
}
