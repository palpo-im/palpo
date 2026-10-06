//! Limited native bootstrap capabilities. A capability stages a request; it
//! cannot authenticate a Matrix user or approve an association.
use palpo_hagency_contract::MatrixUserId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::api::App;
use crate::associations::Intent;
use crate::workflow::Workflows;
use crate::{Result, digest, fail, now_ms};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Pairing {
    pub secret_digest: String,
    pub owner_expires_at_ms: u64,
    pub expires_at_ms: u64,
    pub owner_confirmed_at_ms: Option<u64>,
}
fn capability(token: &str) -> Result<String> {
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(fail(401, "pairing_capability_required"));
    }
    digest(&json!({"pairing":token}))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Start {
    owner_mxid: MatrixUserId,
    intent: Intent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Poll {
    action_id: String,
    /// Once installed locally, polling never needs the profile again.
    #[serde(default)]
    imported: bool,
}
impl App {
    pub(crate) async fn start_pairing(&self, token: &str, input: Value) -> Result<Value> {
        let hash = capability(token)?;
        let input: Start = serde_json::from_value(input)?;
        if input.intent.existing_fleet_id.is_some() {
            return Err(fail(400, "pairing_requires_new_engagement"));
        }
        let admin = self
            .association_admin
            .as_ref()
            .ok_or_else(|| fail(501, "association_admin_unconfigured"))?;
        if self.transport_origin.is_none() || self.relay_origin.is_none() {
            return Err(fail(501, "outbound_unconfigured"));
        }
        let _queue = self.mutation.lock().await;
        let now = now_ms();
        self.store.lock().await.transaction(|state| {
            let mut w = Workflows::load(state)?;
            // Expired unconfirmed requests stop producing notifications and no
            // longer occupy the owner's finite pending-request allowance.
            for a in w.associations.values_mut() {
                if matches!(a.state.as_str(), "awaiting_owner" | "requested") && a.pairing.as_ref().is_some_and(|p|
                    p.expires_at_ms <= now || p.owner_confirmed_at_ms.is_none() && p.owner_expires_at_ms <= now) {
                    a.state = "expired".into(); a.execution = "done".into();
                    a.revision += 1; a.updated_at = now;
                    for n in w.notices.values_mut().filter(|n| n["actionId"] == a.id) {
                        n["cancelled"] = json!(true);
                    }
                }
            }
            let key = digest(&json!({"kind":"association","owner":input.owner_mxid,"requestId":input.intent.request_id}))?;
            let id = format!("action_{}", &key[..32]);
            let exists = w.associations.contains_key(&id);
            if let Some(a) = w.associations.get(&id) {
                let p = a.pairing.as_ref().ok_or_else(|| fail(409, "idempotency_conflict"))?;
                if !bool::from(p.secret_digest.as_bytes().ct_eq(hash.as_bytes())) {
                    return Err(fail(409, "idempotency_conflict"));
                }
            }
            w.request_association(input.intent, &input.owner_mxid, admin, self.matrix.server(), now)?;
            if !exists {
                let a = w.associations.get_mut(&id).unwrap();
                a.state = "awaiting_owner".into();
                a.pairing = Some(Pairing { secret_digest: hash,
                    owner_expires_at_ms: now + 30 * 60_000,
                    expires_at_ms: (now + 7 * 86_400_000).min(a.intent.delegation_expires_at_ms),
                    owner_confirmed_at_ms: None });
                // request_association and this replacement are one transaction:
                // no administrator is notified before owner confirmation.
                w.notices.retain(|_, n| n["actionId"] != id);
                w.notify_association(&id, now)?;
            }
            let a = &w.associations[&id];
            let result = json!({"actionId":id,"fleetId":a.fleet_id,"serverName":self.matrix.server(),
                "serverOrigin":self.public_origin});
            w.save(state)?;
            Ok(result)
        })
    }
    pub(crate) async fn pairing_status(&self, token: &str, input: Value) -> Result<Value> {
        let hash = capability(token)?;
        let input: Poll = serde_json::from_value(input)?;
        let _queue = self.mutation.lock().await;
        let state = self.store.lock().await.read()?;
        let w = Workflows::load(&state)?;
        let a = w
            .associations
            .get(&input.action_id)
            .ok_or_else(|| fail(404, "pairing_not_found"))?;
        let p = a
            .pairing
            .as_ref()
            .ok_or_else(|| fail(404, "pairing_not_found"))?;
        if !bool::from(p.secret_digest.as_bytes().ct_eq(hash.as_bytes())) {
            return Err(fail(404, "pairing_not_found"));
        }
        let now = now_ms();
        let fleet = &state["fleets"][&a.fleet_id];
        let e = w.authority.engagements.get(&a.fleet_id);
        let expired = p.expires_at_ms <= now
            || a.intent.delegation_expires_at_ms <= now
            || p.owner_confirmed_at_ms.is_none() && p.owner_expires_at_ms <= now;
        // Credential rotation or changed delegation terminates bootstrap; the
        // old pairing capability never gains authority over a new generation.
        let unavailable = e.is_some_and(|e| {
            u64::from(e.registration_generation) != 1
                || u64::from(e.delegation_revision) != 1
                || matches!(
                    e.state,
                    palpo_hagency_contract::EngagementState::Suspended
                        | palpo_hagency_contract::EngagementState::Revoked
                )
        }) || fleet["transport"]["generation"]
            .as_u64()
            .is_some_and(|g| g != 1)
            || fleet["registrationGeneration"]
                .as_u64()
                .is_some_and(|g| g != 1)
            || fleet["pendingAdminOperation"].is_string()
            || matches!(fleet["state"].as_str(), Some("paused" | "revoked"));
        let phase = if expired {
            "expired"
        } else if unavailable {
            "unavailable"
        } else if a.state == "awaiting_owner" {
            "awaiting_owner"
        } else if a.state == "requested" {
            "awaiting_admin"
        } else if a.state == "rejected" {
            "rejected"
        } else if a.execution == "setup_failed" {
            "setup_failed"
        } else if a.execution == "verified" {
            "connected"
        } else {
            "awaiting_connection"
        };
        let mut result = json!({"actionId":a.id,"fleetId":a.fleet_id,"phase":phase,
            "ownerMxid":a.owner_mxid,"coordinatorMxid":a.intent.coordinator_mxid,
            "expiresAtMs":if p.owner_confirmed_at_ms.is_some(){p.expires_at_ms}else{p.owner_expires_at_ms}});
        if !input.imported
            && !expired
            && !unavailable
            && p.owner_confirmed_at_ms.is_some()
            && a.state == "approved"
            && fleet["installation"] == "installed"
            && matches!(
                fleet["state"].as_str(),
                Some("ready" | "pending_connection")
            )
            && e.is_some()
        {
            // Native-to-native only. Never included in the Inbox, notifications,
            // browser console response, or logs.
            result["profile"] = json!({"schemaVersion":1,"fleetId":a.fleet_id,"serverName":self.matrix.server(),
                "serverOrigin":self.public_origin,"runtimeId":a.intent.runtime_id,"credentialVersion":1,
                "registration":fleet["registration"],"transport":{"mode":"outbound","url":fleet["transport"]["url"],"token":fleet["transport"]["token"],"generation":fleet["transport"]["generation"]},"engagement":e});
        }
        Ok(result)
    }
}
