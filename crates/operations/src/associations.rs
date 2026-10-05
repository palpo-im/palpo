//! Hagency owner initiated server associations. These share the durable Inbox
//! and notification projection with project requests, but have distinct admin
//! authority and never confer a resource or project approval role.
use palpo_hagency_contract::{
    DefinitionDigest, MatrixUserId, RequestId, ServerEngagementId, ServerName,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::App;
use crate::workflow::Workflows;
use crate::{Result, digest, fail};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Intent {
    pub request_id: RequestId,
    /// Owner-initiated protocol upgrade preserves an installed legacy namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_fleet_id: Option<ServerEngagementId>,
    pub name: String,
    pub runtime_id: DefinitionDigest,
    pub coordinator_mxid: MatrixUserId,
    pub delegation_expires_at_ms: u64,
    #[serde(default)]
    pub allow_self_approval: bool,
    /// Explicit owner authorization for profile retrieval; admin retrieval is
    /// separately bound to the designated current server administrator.
    #[serde(default)]
    pub export_mxids: Vec<MatrixUserId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Association {
    pub id: String,
    pub fleet_id: String,
    pub owner_mxid: MatrixUserId,
    pub administrator_mxid: MatrixUserId,
    pub server_name: ServerName,
    pub intent: Intent,
    pub fingerprint: String,
    pub revision: u64,
    pub state: String,
    pub execution: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub decision: Option<Value>,
    #[serde(default)]
    pub last_error: Option<String>,
}
impl Association {
    pub fn view(&self, actor: &MatrixUserId, now: u64) -> Result<Value> {
        if actor != &self.owner_mxid
            && actor != &self.administrator_mxid
            && actor != &self.intent.coordinator_mxid
        {
            return Err(fail(404, "action_not_found"));
        }
        let review = self.state == "requested"
            && actor == &self.administrator_mxid
            && self.intent.delegation_expires_at_ms > now;
        let export = self.state == "approved"
            && self.intent.delegation_expires_at_ms > now
            && matches!(self.execution.as_str(), "verifying" | "verified")
            && (actor == &self.administrator_mxid || self.intent.export_mxids.contains(actor));
        Ok(
            json!({"id":self.id,"kind":"association","ownerMxid":self.owner_mxid,"fleetId":self.fleet_id,
            "revision":self.revision,"state":self.state,"execution":self.execution,"createdAt":self.created_at,"updatedAt":self.updated_at,"decision":self.decision,
            "needsMyAction":review,"canDecide":review,"canContinue":export,
            "canConnect":self.state=="approved" && matches!(self.execution.as_str(),"verifying"|"verified") && actor==&self.owner_mxid && self.intent.delegation_expires_at_ms>now,
            "canRetrySetup":self.state=="approved" && self.execution=="setup_failed" && actor==&self.administrator_mxid,
            "nextAction":if review {Some("review")}else if export {Some("export_and_connect")}else{None},
            "payload":{"name":self.intent.name,"reason":if self.intent.existing_fleet_id.is_some() {"Upgrade the existing Hagency connection to coordinator approvals. Keep its Matrix identity, credentials and agent history."}else{"Authorize this Hagency runtime to connect to the homeserver. Resource allocations and project decisions remain separate."},"existingFleetId":self.intent.existing_fleet_id,
                "coordinatorMxid":self.intent.coordinator_mxid,"runtimeId":self.intent.runtime_id,"exportMxids":self.intent.export_mxids,
                "delegationExpiresAtMs":self.intent.delegation_expires_at_ms,"allowSelfApproval":self.intent.allow_self_approval},
            "result":{"fleetId":self.fleet_id,"lastError":self.last_error}}),
        )
    }
}

impl Workflows {
    pub(crate) fn association_view(
        &self,
        association: &Association,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let mut current = association.clone();
        if let Some(e) = self.authority.engagements.get(&association.fleet_id) {
            current.intent.coordinator_mxid = e.coordinator.clone();
            current.intent.delegation_expires_at_ms = e.delegation_expires_at_ms;
            current.intent.allow_self_approval = e.allow_self_approval;
            if let Some(exports) = self.engagement_exports.get(&association.fleet_id) {
                current.intent.export_mxids = exports.clone();
            }
            if matches!(
                e.state,
                palpo_hagency_contract::EngagementState::Suspended
                    | palpo_hagency_contract::EngagementState::Revoked
            ) {
                current.execution = serde_json::to_value(e.state)?
                    .as_str()
                    .unwrap_or("unknown")
                    .into();
            }
        }
        current.view(actor, now)
    }
    /// Invoked only by the native owner setup route after Matrix authentication.
    /// No caller-supplied owner/admin or approval state crosses this boundary.
    pub fn request_association(
        &mut self,
        intent: Intent,
        owner: &MatrixUserId,
        administrator: &MatrixUserId,
        server: &ServerName,
        now: u64,
    ) -> Result<Value> {
        if !owner.belongs_to(server)
            || !administrator.belongs_to(server)
            || !intent.coordinator_mxid.belongs_to(server)
            || intent.name.trim().is_empty()
            || intent.name.len() > 256
            || intent.name.chars().any(char::is_control)
            || intent.delegation_expires_at_ms <= now
            || intent.delegation_expires_at_ms > now.saturating_add(366 * 86400000)
            || intent.export_mxids.len() > 2
            || intent
                .export_mxids
                .iter()
                .any(|user| user != owner && user != &intent.coordinator_mxid)
            || intent
                .export_mxids
                .iter()
                .map(MatrixUserId::as_str)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != intent.export_mxids.len()
        {
            return Err(fail(400, "invalid_association_request"));
        }
        let key =
            digest(&json!({"kind":"association","owner":owner,"requestId":intent.request_id}))?;
        let id = format!("action_{}", &key[..32]);
        let fingerprint = digest(&json!({"owner":owner,"server":server,"intent":intent}))?;
        if let Some(existing) = self.associations.get(&id) {
            if existing.fingerprint != fingerprint {
                return Err(fail(409, "idempotency_conflict"));
            }
            return self.association_view(existing, owner, now);
        }
        if self.associations.len() >= 10000
            || self
                .associations
                .values()
                .filter(|a| a.owner_mxid == *owner && a.state == "requested")
                .count()
                >= 20
        {
            return Err(fail(429, "association_limit"));
        }
        let fleet_id = intent
            .existing_fleet_id
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .unwrap_or_else(|| format!("hf_{}", &key[..32]));
        if !valid_legacy_fleet_id(&fleet_id)
            || self.associations.values().any(|a| a.fleet_id == fleet_id)
        {
            return Err(fail(409, "engagement_already_registered"));
        }
        if self.authority.engagements.contains_key(&fleet_id) {
            return Err(fail(409, "engagement_already_registered"));
        }
        self.associations.insert(
            id.clone(),
            Association {
                id: id.clone(),
                fleet_id,
                owner_mxid: owner.clone(),
                administrator_mxid: administrator.clone(),
                server_name: server.clone(),
                intent,
                fingerprint,
                revision: 1,
                state: "requested".into(),
                execution: "pending".into(),
                created_at: now,
                updated_at: now,
                decision: None,
                last_error: None,
            },
        );
        self.notify_association(&id, now)?;
        self.view(&id, owner, now)
    }
    pub(crate) fn notify_association(&mut self, id: &str, now: u64) -> Result<()> {
        let a = self
            .associations
            .get(id)
            .ok_or_else(|| fail(404, "action_not_found"))?;
        for notice in self
            .notices
            .values_mut()
            .filter(|n| n["actionId"] == id && n["revision"] != a.revision)
        {
            notice["cancelled"] = json!(true);
        }
        for recipient in [
            &a.owner_mxid,
            &a.administrator_mxid,
            self.authority
                .engagements
                .get(&a.fleet_id)
                .map_or(&a.intent.coordinator_mxid, |e| &e.coordinator),
        ] {
            let key = format!(
                "{}_{}_{}",
                id,
                a.revision,
                &digest(&json!(recipient))?[..16]
            );
            self.notices.entry(key.clone()).or_insert_with(||json!({"id":key,"actionId":id,"revision":a.revision,"recipient":recipient,
                "createdAt":now,"dueAt":now,"attempt":0,"delivered":0,"seenAt":null,"cancelled":false}));
        }
        Ok(())
    }
}

impl App {
    pub(crate) async fn request_association(
        &self,
        matrix_token: &str,
        input: Value,
    ) -> Result<Value> {
        let admin = self
            .association_admin
            .as_ref()
            .ok_or_else(|| fail(501, "association_admin_unconfigured"))?;
        if self.transport_origin.is_none() || self.relay_origin.is_none() {
            return Err(fail(501, "outbound_unconfigured"));
        }
        let intent: Intent = serde_json::from_value(input)?;
        self.matrix.authenticate(matrix_token).await?;
        let _queue = self.mutation.lock().await;
        let actor = self.matrix.authenticate(matrix_token).await?.user;
        self.store.lock().await.transaction(|state|{
            let mut w=Workflows::load(state)?;
            if let Some(fleet)=&intent.existing_fleet_id {
                let legacy=&state["fleets"][fleet.as_str()];
                if legacy["ownerMxid"]!=actor.as_str() {return Err(fail(404,"engagement_not_found"));}
                let replay=w.associations.values().any(|a|a.fleet_id==fleet.as_str() && a.owner_mxid==actor && a.intent.request_id==intent.request_id);
                if !replay { validate_legacy_fleet(legacy,fleet.as_str(),&actor,self.transport_origin.as_deref().unwrap_or_default())?; }
            }
            let view=w.request_association(intent,&actor,admin,self.matrix.server(),crate::now_ms())?;
            w.save(state)?;
            Ok(json!({"action":view,"serverName":self.matrix.server(),"serverOrigin":self.public_origin}))
        })
    }
}

fn valid_legacy_fleet_id(id: &str) -> bool {
    id.len() == 35 && id.starts_with("hf_") && id[3..].bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn validate_legacy_fleet(
    fleet: &Value,
    id: &str,
    owner: &MatrixUserId,
    origin: &str,
) -> Result<()> {
    if !valid_legacy_fleet_id(id)
        || fleet["id"] != id
        || fleet["ownerMxid"] != owner.as_str()
        || fleet["installation"] != "installed"
        || !matches!(
            fleet["state"].as_str(),
            Some("ready" | "registered" | "verifying" | "offline")
        )
        || fleet["transport"]["mode"] != "outbound"
        || fleet["transport"]["url"] != format!("{origin}/api/fleet/v2/{id}")
        || fleet["transport"]["generation"]
            .as_u64()
            .is_none_or(|v| v == 0)
        || fleet["transport"]["token"].as_str().is_none()
        || fleet["registration"]["id"] != id
        || fleet["pendingAdminOperation"].is_string()
        || fleet["associationId"].is_string()
    {
        return Err(fail(409, "legacy_engagement_unavailable"));
    }
    Ok(())
}
