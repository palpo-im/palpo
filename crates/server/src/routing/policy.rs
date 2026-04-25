use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::policy::sign_event::{PolicySignEventReqBody, PolicySignEventResBody};
use crate::core::serde::CanonicalJsonObject;
use crate::core::signatures::KeyPair;
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
fn sign_event(
    _aa: AuthArgs,
    body: JsonBody<PolicySignEventReqBody>,
) -> JsonResult<PolicySignEventResBody> {
    let signature = sign_policy_event(&body.0.0)?;

    json_ok(PolicySignEventResBody::new(
        config::get().server_name.clone(),
        signature,
    ))
}

fn sign_policy_event(pdu: &serde_json::value::RawValue) -> AppResult<String> {
    let mut object: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|_| {
        MatrixError::bad_json("Policy Server signing request must be a JSON object")
    })?;

    if object.get("type").and_then(|value| value.as_str()) == Some("m.room.policy")
        && object.get("state_key").and_then(|value| value.as_str()) == Some("")
    {
        return Err(MatrixError::forbidden(
            "Policy Server configuration events must not be signed by this endpoint",
            None,
        )
        .into());
    }

    object.remove("signatures");
    object.remove("unsigned");

    let canonical_json = serde_json::to_string(&object)?;
    Ok(config::keypair().sign(canonical_json.as_bytes()).base64())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::sign_policy_event;
    use crate::core::serde::to_raw_json_value;

    #[test]
    fn policy_config_event_is_rejected() {
        let pdu = to_raw_json_value(&json!({
            "type": "m.room.policy",
            "state_key": "",
            "content": {}
        }))
        .unwrap();

        assert!(sign_policy_event(&pdu).is_err());
    }
}
