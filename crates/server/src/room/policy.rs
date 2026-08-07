//! Policy Server support ([MSC4284]).
//!
//! A room opts in to a Policy Server through an `m.room.policy` state event with an empty
//! state key. Every other event in such a room must carry an `ed25519:policy_server`
//! signature from the configured server before it is shown to users; the local server asks
//! the Policy Server for that signature when it is missing.
//!
//! [MSC4284]: https://github.com/matrix-org/matrix-spec-proposals/pull/4284

use crate::core::SigningKeyAlgorithm;
use crate::core::events::StaticEventContent;
use crate::core::events::room::policy::RoomPolicyEventContent;
use crate::core::federation::policy::sign_event::{
    PolicySignEventReqBody, PolicySignEventResBody, sign_event_request,
};
use crate::core::identifiers::*;
use crate::core::room_version_rules::{EventIdFormatVersion, RoomVersionRules};
use crate::core::serde::{
    CanonicalJsonObject, CanonicalJsonValue, canonical_json, to_raw_json_value,
};
use crate::core::signatures::{
    KeyPair, to_canonical_json_string_for_signing, verify_policy_server_signature,
};
use crate::exts::*;
use crate::{AppResult, MatrixError, config};

/// How long to wait for `POST /_matrix/policy/v1/sign` before giving up.
///
/// Sending an event blocks on this round trip, so it is deliberately much shorter than the
/// default federation timeout.
const SIGN_TIMEOUT_SECS: u64 = 30;

/// The error surfaced to clients whose event the Policy Server declined to sign.
const SPAM_MESSAGE: &str = "This message has been rejected as probable spam";

/// Whether the event configures the room's Policy Server.
///
/// Such events are exempt: requiring a signature on them would make it impossible to
/// remove a Policy Server that has stopped signing.
pub fn is_policy_config_event(event_ty: &str, state_key: Option<&str>) -> bool {
    event_ty == RoomPolicyEventContent::TYPE && state_key == Some("")
}

/// Reads the room's usable Policy Server configuration, if it has one.
///
/// `Ok(None)` means "this room does not use a Policy Server", which the MSC says is how an
/// invalid configuration must behave:
///
/// * no `m.room.policy` state event with an empty state key,
/// * content that does not parse, or that carries no `ed25519` public key,
/// * `via` is not joined to the room.
///
/// An operational failure -- a database error, say -- comes back as an error rather than
/// as `None`. Reading it as "no Policy Server" would let a transient fault switch off
/// moderation for its duration, which is the one failure mode this must not have.
pub async fn policy_server(room_id: &RoomId) -> AppResult<Option<RoomPolicyEventContent>> {
    let event =
        match crate::room::get_state(room_id, &RoomPolicyEventContent::TYPE.into(), "", None).await
        {
            Ok(event) => event,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e),
        };

    let Ok(content) = event.get_content::<RoomPolicyEventContent>() else {
        debug!(%room_id, "room policy event content is not usable");
        return Ok(None);
    };

    if !content
        .public_keys
        .contains_key(&SigningKeyAlgorithm::Ed25519)
    {
        return Ok(None);
    }
    if !crate::room::is_server_joined(&content.via, room_id).await? {
        return Ok(None);
    }

    Ok(Some(content))
}

/// Whether this server is the room's Policy Server.
///
/// A room may legitimately name us in `via`: we advertise a policy key at
/// `/.well-known/matrix/policy_server` and serve `/_matrix/policy/v1/sign`. Signing such an
/// event is a local operation, so it neither needs nor should make a federation request to
/// ourselves -- but it does still have to happen, or a room that named us would get no
/// enforcement at all.
fn is_local_policy_server(policy: &RoomPolicyEventContent) -> bool {
    policy.via == *config::server_name()
}

/// Signs an event with this server's Policy Server key.
///
/// Shared with the `/_matrix/policy/v1/sign` endpoint, so a signature we produce for
/// ourselves is byte-for-byte the one a peer would have been given.
pub fn sign_locally(pdu_json: &CanonicalJsonObject, rules: &RoomVersionRules) -> AppResult<String> {
    let redacted = canonical_json::redact(pdu_json.clone(), &rules.redaction, None)
        .map_err(|e| MatrixError::bad_json(format!("event could not be redacted: {e}")))?;
    let canonical = to_canonical_json_string_for_signing(&redacted)
        .map_err(|e| MatrixError::bad_json(format!("event is not canonical JSON: {e}")))?;
    Ok(config::keypair().sign(canonical.as_bytes()).base64())
}

/// The event JSON that a Policy Server signature covers.
///
/// Signatures are computed over the PDU, which for every room version after v2 does not
/// carry `event_id`. Palpo keeps `event_id` in the stored JSON, so strip it here rather
/// than at each call site.
fn signable_pdu(pdu_json: &CanonicalJsonObject, rules: &RoomVersionRules) -> CanonicalJsonObject {
    let mut pdu_json = pdu_json.clone();
    if rules.event_id_format != EventIdFormatVersion::V1 {
        pdu_json.remove("event_id");
    }
    pdu_json
}

/// Whether `pdu_json` already carries a valid signature from `policy`.
fn has_valid_signature(
    policy: &RoomPolicyEventContent,
    pdu_json: &CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> bool {
    match verify_policy_server_signature(policy, &signable_pdu(pdu_json, rules), rules) {
        Ok(()) => true,
        Err(e) => {
            debug!(
                policy_server = %policy.via,
                error = %e,
                "event is not validly signed by the room's policy server"
            );
            false
        }
    }
}

/// Adds the Policy Server's signature for `pdu_json` to its `signatures` block.
///
/// Returns an error when the Policy Server refuses to sign (the event is spam), cannot be
/// reached, or returns a signature that does not verify against the key in the room's
/// `m.room.policy` event.
async fn add_signature(
    policy: &RoomPolicyEventContent,
    pdu_json: &mut CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<()> {
    let signable = signable_pdu(pdu_json, rules);
    let signature = if is_local_policy_server(policy) {
        sign_locally(&signable, rules)?
    } else {
        let body = PolicySignEventReqBody::new(to_raw_json_value(&signable)?);
        let request = sign_event_request(&policy.via.origin().await, body)?.into_inner();
        let res_body =
            crate::sending::send_federation_request(&policy.via, request, Some(SIGN_TIMEOUT_SECS))
                .await?
                .json::<PolicySignEventResBody>()
                .await?;

        match res_body.ed25519_signature(&policy.via) {
            Some(signature) => signature.to_owned(),
            None => return Err(MatrixError::forbidden(SPAM_MESSAGE, None).into()),
        }
    };

    // Merge rather than replace: the sender's own signature must survive, otherwise the
    // event fails authorization everywhere.
    let signatures = match pdu_json
        .entry("signatures".to_owned())
        .or_insert_with(|| CanonicalJsonValue::Object(Default::default()))
    {
        CanonicalJsonValue::Object(signatures) => signatures,
        _ => return Err(MatrixError::bad_json("signatures must be a JSON object").into()),
    };
    let entry = match signatures
        .entry(policy.via.to_string())
        .or_insert_with(|| CanonicalJsonValue::Object(Default::default()))
    {
        CanonicalJsonValue::Object(entry) => entry,
        _ => return Err(MatrixError::bad_json("signatures must be a JSON object").into()),
    };
    entry.insert(
        PolicySignEventResBody::POLICY_SERVER_ED25519_SIGNING_KEY_ID.to_owned(),
        CanonicalJsonValue::String(signature),
    );

    if !has_valid_signature(policy, pdu_json, rules) {
        return Err(MatrixError::unknown(format!(
            "policy server {} failed to sign event correctly",
            policy.via
        ))
        .into());
    }

    Ok(())
}

/// Checks `pdu_json` against the room's Policy Server, fetching a signature when needed.
///
/// `pdu_json` is updated in place with any signature obtained from the Policy Server so
/// that callers persist it and pass it on transitively.
///
/// Returns `Ok(())` when the room has no usable Policy Server, when the event configures
/// the Policy Server, or when the event is signed. Any other outcome is an error: callers
/// on the local send path turn it into a client-visible rejection, callers on the
/// federation path soft fail the event.
pub async fn check_event(
    room_id: &RoomId,
    pdu_json: &mut CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<()> {
    let event_ty = match pdu_json.get("type") {
        Some(CanonicalJsonValue::String(event_ty)) => event_ty.as_str(),
        _ => return Err(MatrixError::bad_json("pdu has no valid type field").into()),
    };
    let state_key = match pdu_json.get("state_key") {
        Some(CanonicalJsonValue::String(state_key)) => Some(state_key.as_str()),
        _ => None,
    };
    if is_policy_config_event(event_ty, state_key) {
        return Ok(());
    }

    let Some(policy) = policy_server(room_id).await? else {
        return Ok(());
    };

    if has_valid_signature(&policy, pdu_json, rules) {
        return Ok(());
    }

    add_signature(&policy, pdu_json, rules).await
}

/// Verify an event received by a synchronous federation membership endpoint, then enforce
/// the room's Policy Server.
///
/// The generic transaction path already verifies the sender before reaching the policy
/// check. The invite/join/leave/knock endpoints need a hard Policy Server error instead of
/// a soft failure, so they call this earlier and must perform the same verification first;
/// otherwise an authenticated peer could make us forward unverified events to the Policy
/// Server.
pub async fn check_federation_event(
    room_id: &RoomId,
    pdu_json: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> AppResult<()> {
    crate::server_key::verify_event(pdu_json, room_version)
        .await
        .map_err(|e| MatrixError::invalid_param(format!("signature verification failed: {e}")))?;
    let rules = crate::room::get_version_rules(room_version)?;
    check_event(room_id, pdu_json, &rules).await
}

/// Whether the event is allowed into the room by the room's Policy Server.
///
/// The federation variant of [`check_event`]: a Policy Server that refuses to sign or
/// cannot be reached means the event must not reach clients, but it is not a protocol
/// violation by the sending server, so it is reported as a soft failure rather than an
/// error.
pub async fn is_event_allowed(
    room_id: &RoomId,
    pdu_json: &mut CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> bool {
    match check_event(room_id, pdu_json, rules).await {
        Ok(()) => true,
        Err(e) => {
            warn!(%room_id, error = %e, "event was not allowed by the room's policy server");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_policy_config_event, signable_pdu};
    use crate::core::room_version_rules::RoomVersionRules;
    use crate::core::serde::CanonicalJsonObject;

    #[test]
    fn policy_configuration_event_is_exempt() {
        assert!(is_policy_config_event("m.room.policy", Some("")));
        // A non-empty state key is a different event and is not exempt.
        assert!(!is_policy_config_event("m.room.policy", Some("@a:b.org")));
        assert!(!is_policy_config_event("m.room.policy", None));
        assert!(!is_policy_config_event("m.room.message", Some("")));
    }

    #[test]
    fn signable_pdu_drops_event_id_for_hashed_event_ids() {
        let pdu_json: CanonicalJsonObject = serde_json::from_str(
            r#"{"event_id": "$abc", "type": "m.room.message", "content": {}}"#,
        )
        .unwrap();

        // Room version 1 puts `event_id` in the PDU, so it is part of what gets signed.
        assert!(signable_pdu(&pdu_json, &RoomVersionRules::V1).contains_key("event_id"));
        // Every later room version computes the event ID from the event, so palpo's stored
        // copy of it must not leak into the signature base.
        assert!(!signable_pdu(&pdu_json, &RoomVersionRules::V11).contains_key("event_id"));
    }
}
