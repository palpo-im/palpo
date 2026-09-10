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
    CanonicalJsonObject, CanonicalJsonValue, RawJsonValue, canonical_json, to_raw_json_value,
};
use crate::core::signatures::{
    KeyPair, to_canonical_json_string_for_signing, verify_policy_server_signature,
};
use crate::core::state::Event;
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

fn event_kind_and_state_key(pdu_json: &CanonicalJsonObject) -> AppResult<(&str, Option<&str>)> {
    let event_ty = match pdu_json.get("type") {
        Some(CanonicalJsonValue::String(event_ty)) => event_ty.as_str(),
        _ => return Err(MatrixError::bad_json("pdu has no valid type field").into()),
    };
    let state_key = match pdu_json.get("state_key") {
        Some(CanonicalJsonValue::String(state_key)) => Some(state_key.as_str()),
        _ => None,
    };
    Ok((event_ty, state_key))
}

async fn check_against_policy(
    policy: &RoomPolicyEventContent,
    pdu_json: &mut CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<()> {
    if has_valid_signature(policy, pdu_json, rules) {
        return Ok(());
    }
    add_signature(policy, pdu_json, rules).await
}

/// Read a policy configuration from the full state bundled with a federation invite.
///
/// This state is used only when the local server is not joined and consequently has no
/// trusted current state of its own. The matching state event must itself have a valid
/// Matrix signature before its public key is used.
async fn policy_from_invite_state(
    room_id: &RoomId,
    invite_room_state: &[Box<RawJsonValue>],
    room_version: &RoomVersionId,
) -> AppResult<Option<RoomPolicyEventContent>> {
    let mut candidate = None;
    for raw in invite_room_state {
        let event: CanonicalJsonObject = serde_json::from_str(raw.get())
            .map_err(|_| MatrixError::invalid_param("invite state event is invalid JSON"))?;
        let (event_ty, state_key) = event_kind_and_state_key(&event)?;
        if !is_policy_config_event(event_ty, state_key) {
            continue;
        }
        if let Some(CanonicalJsonValue::String(event_room_id)) = event.get("room_id")
            && event_room_id != room_id.as_str()
        {
            return Err(MatrixError::invalid_param(
                "invite room policy belongs to a different room",
            )
            .into());
        }
        if candidate.is_some() {
            return Err(MatrixError::invalid_param(
                "invite room state contains multiple policy events",
            )
            .into());
        }
        match crate::server_key::verify_event(&event, room_version).await {
            Ok(crate::core::signatures::Verified::All) => {}
            Ok(crate::core::signatures::Verified::Signatures) => {
                return Err(MatrixError::invalid_param(
                    "invite room policy event has an invalid content hash",
                )
                .into());
            }
            Err(e) => {
                return Err(MatrixError::invalid_param(format!(
                    "invite room policy signature verification failed: {e}"
                ))
                .into());
            }
        }
        candidate = Some(event);
    }

    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let Some(content) = candidate.get("content") else {
        return Ok(None);
    };
    let Ok(policy) = serde_json::from_value::<RoomPolicyEventContent>(content.clone().into())
    else {
        return Ok(None);
    };
    if !policy
        .public_keys
        .contains_key(&SigningKeyAlgorithm::Ed25519)
    {
        return Ok(None);
    }
    // A policy event alone does not prove that its server still participates.
    // First-time invitees can only use a joined membership bundled with it.
    for raw in invite_room_state {
        let event: CanonicalJsonObject = serde_json::from_str(raw.get())?;
        if invite_proves_joined_server(&event, room_id, &policy.via)
            && matches!(
                crate::server_key::verify_event(&event, room_version).await,
                Ok(crate::core::signatures::Verified::All)
            )
        {
            return Ok(Some(policy));
        }
    }
    Ok(None)
}

fn invite_proves_joined_server(
    event: &CanonicalJsonObject,
    room_id: &RoomId,
    server: &ServerName,
) -> bool {
    if event.get("type").and_then(CanonicalJsonValue::as_str) != Some("m.room.member") {
        return false;
    }
    if let Some(value) = event.get("room_id")
        && value.as_str() != Some(room_id.as_str())
    {
        return false;
    }
    let Some(user) = event
        .get("state_key")
        .and_then(CanonicalJsonValue::as_str)
        .and_then(|value| UserId::parse(value).ok())
    else {
        return false;
    };
    user.server_name() == server
        && event
            .get("content")
            .and_then(CanonicalJsonValue::as_object)
            .and_then(|content| content.get("membership"))
            .and_then(CanonicalJsonValue::as_str)
            == Some("join")
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
    let (event_ty, state_key) = event_kind_and_state_key(pdu_json)?;
    if is_policy_config_event(event_ty, state_key) {
        return Ok(());
    }

    let Some(policy) = policy_server(room_id).await? else {
        return Ok(());
    };

    check_against_policy(&policy, pdu_json, rules).await
}

/// Enforce the policy applicable to an incoming federation invite.
///
/// A server already joined to the room uses its trusted current state. On the first invite
/// there is no local state yet, so the signed `m.room.policy` event supplied in
/// `invite_room_state` is the only configuration available to the invitee.
pub async fn check_invite_event(
    room_id: &RoomId,
    pdu_json: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
    invite_room_state: &[Box<RawJsonValue>],
) -> AppResult<()> {
    let (event_ty, state_key) = event_kind_and_state_key(pdu_json)?;
    if is_policy_config_event(event_ty, state_key) {
        return Ok(());
    }

    let rules = crate::room::get_version_rules(room_version)?;
    if let Some(policy) = policy_server(room_id).await? {
        return check_against_policy(&policy, pdu_json, &rules).await;
    }
    if crate::room::is_server_joined(config::server_name(), room_id).await? {
        return Ok(());
    }
    let Some(policy) = policy_from_invite_state(room_id, invite_room_state, room_version).await?
    else {
        return Ok(());
    };
    check_against_policy(&policy, pdu_json, &rules).await
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
    event_id: &EventId,
    pdu_json: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> AppResult<()> {
    crate::server_key::verify_event(pdu_json, room_version)
        .await
        .map_err(|e| MatrixError::invalid_param(format!("signature verification failed: {e}")))?;
    let rules = crate::room::get_version_rules(room_version)?;
    let pdu = crate::event::PduEvent::from_canonical_object(room_id, event_id, pdu_json.clone())
        .map_err(|_| MatrixError::invalid_param("membership event is not a valid PDU"))?;
    crate::event::handler::auth_check(&pdu, &rules, None).await?;
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

/// Complete a deferred policy check after a recovered outlier passes room auth.
/// A refusal remains durable even if another recovery path loads the event again.
pub(crate) async fn check_recovered_event(
    pdu: &crate::event::SnPduEvent,
    json_data: &mut CanonicalJsonObject,
    rules: &RoomVersionRules,
) -> AppResult<()> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    use crate::data::schema::events;
    use crate::event::POLICY_REFUSED_REASON;

    if pdu.rejected() {
        return Err(crate::AppError::internal(
            "cannot recover a rejected backfill",
        ));
    }
    if !is_event_allowed(&pdu.room_id, json_data, rules).await {
        diesel::update(events::table.find(&pdu.event_id))
            .set((
                events::is_rejected.eq(true),
                events::soft_failed.eq(true),
                events::rejection_reason.eq(POLICY_REFUSED_REASON),
            ))
            .execute(&mut crate::data::connect().await?)
            .await?;
        return Err(crate::AppError::internal(POLICY_REFUSED_REASON));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_policy_config_event, signable_pdu};
    use crate::core::room_version_rules::RoomVersionRules;
    use crate::core::serde::CanonicalJsonObject;
    use crate::core::state::Event;

    #[test]
    fn invite_requires_joined_membership_on_the_policy_server() {
        use super::invite_proves_joined_server;
        let room = crate::core::RoomId::parse("!room:example.org").unwrap();
        let server = crate::core::ServerName::parse("policy.example.org").unwrap();
        let original = serde_json::json!({
            "type": "m.room.member", "room_id": "!room:example.org",
            "state_key": "@moderator:policy.example.org", "content": {"membership": "join"}
        });
        let parse = |value| serde_json::from_value::<CanonicalJsonObject>(value).unwrap();
        assert!(invite_proves_joined_server(
            &parse(original.clone()),
            &room,
            &server
        ));
        for (key, value) in [
            ("type", "m.room.policy"),
            ("room_id", "!other:example.org"),
            ("state_key", "@user:other.example.org"),
        ] {
            let mut event = original.clone();
            event[key] = value.into();
            assert!(!invite_proves_joined_server(&parse(event), &room, &server));
        }
        for membership in ["leave", "invite", "ban", "knock"] {
            let mut event = original.clone();
            event["content"]["membership"] = membership.into();
            assert!(!invite_proves_joined_server(&parse(event), &room, &server));
        }
    }

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
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_policy_refused_membership_stays_hidden_after_reprocessing() {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        use crate::data::schema::events;
        use crate::event::{OutlierPdu, PduEvent};
        crate::test_database::init();
        let pdu: PduEvent = serde_json::from_value(serde_json::json!({
            "event_id": "$refused-membership", "room_id": "!policy:example.org",
            "sender": "@moderator:example.org", "state_key": "@victim:example.org",
            "type":"m.room.member", "content":{"membership":"ban"},
            "origin_server_ts":1, "depth":1, "hashes":{"sha256":""}
        }))
        .unwrap();
        let outlier = OutlierPdu {
            json_data: crate::core::serde::to_canonical_object(&pdu).unwrap(),
            room_id: pdu.room_id.clone(),
            pdu,
            remote_server: "example.org".try_into().unwrap(),
            room_version: crate::core::RoomVersionId::V11,
            soft_failed: false,
            policy_refused: true,
            event_sn: None,
        };
        let (stored, mut json_data, _guard) =
            outlier.clone().save_to_database(false).await.unwrap();
        assert!(
            super::check_recovered_event(&stored, &mut json_data, &RoomVersionRules::V11)
                .await
                .is_err()
        );
        assert!(
            crate::event::handler::process_to_timeline_pdu(stored.clone(), json_data.clone(), None)
                .await
                .is_err()
        );
        let victim = crate::core::UserId::parse("@victim:example.org").unwrap();
        assert!(!stored.user_can_see(&victim).await.unwrap());
        let mut conn = crate::data::connect().await.unwrap();
        assert!(
            events::table
                .find(&stored.event_id)
                .select(events::is_rejected)
                .first::<bool>(&mut conn)
                .await
                .unwrap()
        );
        // Missing predecessors are recoverable; they must not inherit the policy
        // refusal of the first event just because both carry soft_failed=true.
        let mut recoverable = outlier.clone();
        recoverable.pdu.event_id = "$recoverable-backfill".try_into().unwrap();
        recoverable.json_data = crate::core::serde::to_canonical_object(&recoverable.pdu).unwrap();
        recoverable.event_sn = None;
        recoverable.policy_refused = false;
        recoverable.soft_failed = true;
        let (backfill, mut json_data, _guard) = recoverable.save_to_database(true).await.unwrap();
        assert!(backfill.soft_failed);
        assert!(!backfill.rejected());
        super::check_recovered_event(&backfill, &mut json_data, &RoomVersionRules::V11)
            .await
            .unwrap();
        // Failure of the deferred check must persist rejection before returning;
        // loading the event again cannot turn that failure into an allowed retry.
        json_data.remove("type");
        assert!(
            super::check_recovered_event(&backfill, &mut json_data, &RoomVersionRules::V11)
                .await
                .is_err()
        );
        let reloaded = crate::room::timeline::get_pdu(&backfill.event_id)
            .await
            .unwrap();
        assert!(reloaded.rejected());
        assert!(reloaded.soft_failed);
        assert!(
            crate::event::handler::process_to_timeline_pdu(reloaded, json_data, None)
                .await
                .is_err()
        );
        diesel::update(events::table.find(&stored.event_id))
            .set(events::is_rejected.eq(false))
            .execute(&mut conn)
            .await
            .unwrap();
        let mut existing = outlier;
        existing.event_sn = Some(stored.event_sn);
        existing.save_to_database(false).await.unwrap();
        assert!(
            events::table
                .find(&stored.event_id)
                .select(events::is_rejected)
                .first::<bool>(&mut conn)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_policy_refusal_survives_a_verdictless_replay() {
        use diesel::prelude::*;
        use diesel_async::RunQueryDsl;

        use crate::data::schema::events;
        use crate::event::{OutlierPdu, POLICY_REFUSED_REASON, PduEvent};
        crate::test_database::init();
        let member = |event_id: &str, ts: u64| -> PduEvent {
            serde_json::from_value(serde_json::json!({
                "event_id": event_id, "room_id": "!policy-replay:example.org",
                "sender": "@moderator:example.org", "state_key": "@victim:example.org",
                "type": "m.room.member", "content": {"membership": "ban"},
                "origin_server_ts": ts, "depth": ts, "hashes": {"sha256": ""}
            }))
            .unwrap()
        };
        let outlier = |pdu: PduEvent| OutlierPdu {
            json_data: crate::core::serde::to_canonical_object(&pdu).unwrap(),
            room_id: pdu.room_id.clone(),
            pdu,
            remote_server: "example.org".try_into().unwrap(),
            room_version: crate::core::RoomVersionId::V11,
            soft_failed: false,
            policy_refused: false,
            event_sn: None,
        };
        let mut conn = crate::data::connect().await.unwrap();

        let mut refused = outlier(member("$replayed-refusal", 1));
        refused.policy_refused = true;
        let (stored, ..) = refused.clone().save_to_database(false).await.unwrap();
        assert!(stored.rejected());

        // A peer can re-offer the same event while its predecessors are momentarily
        // missing. `process_to_outlier_pdu` then leaves it unauthorised, which skips the
        // Policy Server request and yields `policy_refused == false` -- the absence of a
        // verdict, not an allow. The stored refusal has to survive that replay.
        let mut replay = refused;
        replay.policy_refused = false;
        replay.soft_failed = true;
        let (replayed, ..) = replay.save_to_database(false).await.unwrap();
        assert!(replayed.rejected(), "a replay promoted a refused event");
        assert_eq!(
            replayed.rejection_reason.as_deref(),
            Some(POLICY_REFUSED_REASON)
        );
        assert!(replayed.soft_failed);
        let (is_rejected, reason) = events::table
            .find(&replayed.event_id)
            .select((events::is_rejected, events::rejection_reason))
            .first::<(bool, Option<String>)>(&mut conn)
            .await
            .unwrap();
        assert!(is_rejected, "a replay cleared is_rejected in the database");
        assert_eq!(reason.as_deref(), Some(POLICY_REFUSED_REASON));
        let victim = crate::core::UserId::parse("@victim:example.org").unwrap();
        assert!(!replayed.user_can_see(&victim).await.unwrap());

        // The opposite direction must keep working: an event first stored without a
        // verdict is still rejected once a later pass actually obtains one.
        let mut pending = outlier(member("$refused-on-replay", 2));
        pending.soft_failed = true;
        let (undecided, ..) = pending.clone().save_to_database(false).await.unwrap();
        assert!(!undecided.rejected());
        pending.soft_failed = false;
        pending.policy_refused = true;
        let (now_refused, ..) = pending.save_to_database(false).await.unwrap();
        assert!(now_refused.rejected());
        assert!(
            events::table
                .find(&now_refused.event_id)
                .select(events::is_rejected)
                .first::<bool>(&mut conn)
                .await
                .unwrap()
        );
    }
}
