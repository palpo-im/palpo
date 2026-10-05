//! Association approval, idempotent appservice installation and native profile
//! export. Secrets stay in the private store and the host's save-file response.
use palpo_hagency_contract::{CommandId, MatrixUserId, ServerEngagement, authorize_association};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::App;
use crate::associations::Association;
use crate::workflow::Workflows;
use crate::{Result, digest, fail, now_ms, secret};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Decision {
    id: String,
    decision: String,
    expected_revision: u64,
    command_id: CommandId,
    #[serde(default)]
    reason: String,
}

impl App {
    pub(crate) async fn decide_association(&self, bearer: &str, input: Value) -> Result<Value> {
        let decision: Decision = serde_json::from_value(input.clone())?;
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let now = now_ms();
        let digest = digest(&json!({"actor":identity.user,"decision":input}))?;
        let relay = self
            .relay_origin
            .as_ref()
            .ok_or_else(|| fail(501, "outbound_unconfigured"))?;
        let transport = self
            .transport_origin
            .as_ref()
            .ok_or_else(|| fail(501, "outbound_unconfigured"))?;
        let association = Workflows::load(&self.store.lock().await.read()?)?
            .associations
            .get(&decision.id)
            .cloned()
            .ok_or_else(|| fail(404, "action_not_found"))?;
        authorize_association(
            &identity.user,
            &association.administrator_mxid,
            self.matrix.server(),
            identity.admin,
        )?;
        if !matches!(decision.decision.as_str(), "approve" | "reject")
            || decision.reason.len() > 2000
            || decision.reason.chars().any(char::is_control)
            || decision.decision == "reject" && decision.reason.trim().is_empty()
        {
            return Err(fail(400, "invalid_decision"));
        }
        if decision.decision == "approve" {
            for user in [
                &association.owner_mxid,
                &association.intent.coordinator_mxid,
            ] {
                self.active_human(user, &session.token).await?;
            }
        }
        self.authenticate(bearer).await?;
        let approved=self.store.lock().await.transaction(|state|{
            let mut w=Workflows::load(state)?;
            let receipt_key=format!("association_{}",decision.command_id.as_str());
            if let Some(receipt)=w.receipts.get(&receipt_key) {
                if receipt["digest"]!=digest {return Err(fail(409,"idempotency_conflict"));}
                return Ok(w.associations[&decision.id].state=="approved");
            }
            let a=w.associations.get_mut(&decision.id).ok_or_else(||fail(404,"action_not_found"))?;
            if a.revision!=decision.expected_revision || a.state!="requested" || a.intent.delegation_expires_at_ms<=now {
                return Err(fail(409,"association_revision_changed"));
            }
            let approved=decision.decision=="approve";
            a.state=if approved {"approved"}else{"rejected"}.into();
            a.execution=if approved {"configuring"}else{"done"}.into();a.revision+=1;a.updated_at=now;
            a.decision=Some(json!({"by":identity.user,"at":now,"commandId":decision.command_id,"reason":decision.reason}));
            if approved {
                if state["fleets"].get(&a.fleet_id).is_some() {return Err(fail(409,"engagement_already_registered"));}
                let registration=registration(a,self.matrix.server().as_str(),relay);
                state["fleets"][&a.fleet_id]=json!({"id":a.fleet_id,"name":a.intent.name,"ownerMxid":a.owner_mxid,"associationId":a.id,
                    "runtimeId":a.intent.runtime_id,"state":"authorized","installation":"pending","credentialVersion":1,"registrationGeneration":1,
                    "registration":registration,"agents":{},"representativeMxid":format!("@{}_representative:{}",a.fleet_id,self.matrix.server().as_str()),
                    "transport":{"mode":"outbound","url":format!("{transport}/api/fleet/v2/{}",a.fleet_id),"token":secret(),"generation":1},
                    "localTaskStop":"unknown","createdAtMs":now});
                let e:ServerEngagement=serde_json::from_value(json!({"id":a.fleet_id,"server":self.matrix.server(),"owner":a.owner_mxid,
                    "coordinator":a.intent.coordinator_mxid,"registrationGeneration":1,"delegationRevision":1,"delegationExpiresAtMs":a.intent.delegation_expires_at_ms,
                    "state":"configuring","allowSelfApproval":a.intent.allow_self_approval,"coordinatorApprovalV1":false}))?;
                w.authority.engagements.insert(a.fleet_id.clone(),e);
            }
            w.receipts.insert(receipt_key,json!({"digest":digest,"actionId":decision.id,"actor":identity.user,"decision":decision.decision}));
            w.notify_association(&decision.id,now)?;w.save(state)?;
            state["audit"].as_array_mut().ok_or_else(||fail(503,"workflow_state_invalid"))?.push(json!({"atMs":now,"actor":identity.user,"action":"association.decide","actionId":decision.id,"decision":decision.decision}));
            Ok(approved)
        })?;
        if approved {
            self.install_association(&association, &session.token)
                .await?;
        }
        Ok(
            json!({"action":Workflows::load(&self.store.lock().await.read()?)?.view(&decision.id,&identity.user,now_ms())?}),
        )
    }

    async fn active_human(&self, user: &MatrixUserId, admin_token: &str) -> Result<()> {
        let record = self
            .matrix
            .segments(
                Method::GET,
                &["_palpo", "admin", "v2", "users", user.as_str()],
                admin_token,
                None,
                None,
            )
            .await?;
        if record["deactivated"] == true
            || record["locked"] == true
            || record["appservice_id"].as_str().is_some()
        {
            return Err(fail(403, "active_human_account_required"));
        }
        Ok(())
    }

    async fn install_association(&self, a: &Association, admin_token: &str) -> Result<()> {
        let fleet = self.store.lock().await.read()?["fleets"][&a.fleet_id].clone();
        if fleet["pendingAdminOperation"].is_string()
            || matches!(fleet["state"].as_str(), Some("paused" | "revoked"))
        {
            return Err(fail(409, "engagement_unavailable"));
        }
        let result = async {
            let actual = match self
                .matrix
                .segments(
                    Method::GET,
                    &["_palpo", "admin", "v1", "appservices", &a.fleet_id],
                    admin_token,
                    None,
                    None,
                )
                .await
            {
                Ok(value) => Some(value),
                Err(e) if e.status == 404 => None,
                Err(e) => return Err(e),
            };
            if actual.is_none() {
                self.namespace_available(&a.fleet_id, admin_token).await?;
                self.matrix
                    .call(
                        Method::POST,
                        "/_palpo/admin/v1/appservices",
                        admin_token,
                        Some(&fleet["registration"]),
                    )
                    .await?;
            }
            let actual = self
                .matrix
                .segments(
                    Method::GET,
                    &["_palpo", "admin", "v1", "appservices", &a.fleet_id],
                    admin_token,
                    None,
                    None,
                )
                .await?;
            if !registration_matches(&actual, &fleet["registration"]) || actual["disabled"] == true
            {
                return Err(fail(409, "registration_drift"));
            }
            let as_token = fleet["registration"]["as_token"]
                .as_str()
                .ok_or_else(|| fail(503, "registration_unavailable"))?;
            for local in ["representative", "approval"] {
                let mxid = format!("@{}_{local}:{}", a.fleet_id, self.matrix.server().as_str());
                let existing = match self
                    .matrix
                    .segments(
                        Method::GET,
                        &["_palpo", "admin", "v2", "users", &mxid],
                        admin_token,
                        None,
                        None,
                    )
                    .await
                {
                    Ok(value) => Some(value),
                    Err(e) if e.status == 404 => None,
                    Err(e) => return Err(e),
                };
                if existing.is_some_and(|u| u["appservice_id"] != a.fleet_id) {
                    return Err(fail(409, "identity_conflict"));
                }
                let who = self
                    .matrix
                    .segments(
                        Method::GET,
                        &["_matrix", "client", "v3", "account", "whoami"],
                        as_token,
                        Some(&mxid),
                        None,
                    )
                    .await?;
                let observed = self
                    .matrix
                    .segments(
                        Method::GET,
                        &["_palpo", "admin", "v2", "users", &mxid],
                        admin_token,
                        None,
                        None,
                    )
                    .await?;
                if who["user_id"] != mxid
                    || observed["appservice_id"] != a.fleet_id
                    || observed["deactivated"] == true
                    || observed["locked"] == true
                {
                    return Err(fail(409, "identity_unverified"));
                }
            }
            Ok::<(), crate::Error>(())
        }
        .await;
        self.store.lock().await.transaction(|state| {
            let mut w = Workflows::load(state)?;
            let row = w
                .associations
                .get_mut(&a.id)
                .ok_or_else(|| fail(404, "action_not_found"))?;
            let was_verified = fleet["state"] == "ready";
            let next = if result.is_ok() {
                if was_verified {
                    "verified"
                } else {
                    "verifying"
                }
            } else {
                "setup_failed"
            };
            if row.execution != next {
                row.revision += 1;
                row.updated_at = now_ms();
            }
            row.execution = next.into();
            row.last_error = result.as_ref().err().map(|e| e.code.into());
            state["fleets"][&a.fleet_id]["installation"] = json!(if result.is_ok() {
                "installed"
            } else {
                "failed"
            });
            state["fleets"][&a.fleet_id]["lastError"] = json!(row.last_error);
            if result.is_ok() {
                state["fleets"][&a.fleet_id]["state"] = json!(if was_verified {
                    "ready"
                } else {
                    "pending_connection"
                });
                w.authority
                    .engagements
                    .get_mut(&a.fleet_id)
                    .ok_or_else(|| fail(503, "engagement_missing"))?
                    .state = if was_verified {
                    palpo_hagency_contract::EngagementState::Verified
                } else {
                    palpo_hagency_contract::EngagementState::Verifying
                };
            }
            w.notify_association(&a.id, now_ms())?;
            w.save(state)
        })?;
        // A durable human decision remains approved when setup must be retried.
        // Its explicit setup_failed state is returned; never request reapproval.
        Ok(())
    }

    async fn namespace_available(&self, id: &str, token: &str) -> Result<()> {
        let list = self
            .matrix
            .call(Method::GET, "/_palpo/admin/v1/appservices", token, None)
            .await?;
        let registrations = list["appservices"]
            .as_array()
            .filter(|a| a.len() <= 10000)
            .ok_or_else(|| fail(502, "invalid_registration_list"))?;
        let prefix = format!("{id}_");
        for row in registrations {
            let other_id = row["id"]
                .as_str()
                .ok_or_else(|| fail(502, "invalid_registration_list"))?;
            if other_id == id {
                continue;
            }
            let other = self
                .matrix
                .segments(
                    Method::GET,
                    &["_palpo", "admin", "v1", "appservices", other_id],
                    token,
                    None,
                    None,
                )
                .await?;
            if other["sender_localpart"]
                .as_str()
                .is_some_and(|s| s.starts_with(&prefix))
            {
                return Err(fail(409, "namespace_conflict"));
            }
            for namespace in other["namespaces"]["users"]
                .as_array()
                .ok_or_else(|| fail(409, "namespace_policy"))?
            {
                let pattern = namespace["regex"]
                    .as_str()
                    .ok_or_else(|| fail(409, "namespace_policy"))?;
                let literal = pattern
                    .strip_prefix("^@")
                    .unwrap_or_default()
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                    .collect::<String>();
                let suffix = pattern.get(2 + literal.len()..).unwrap_or_default();
                if literal.is_empty()
                    || suffix.starts_with(['?', '*', '+', '{'])
                    || pattern.contains('|')
                    || literal.starts_with(&prefix)
                    || prefix.starts_with(&literal)
                {
                    return Err(fail(409, "namespace_policy"));
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn association_fleet(
        &self,
        bearer: &str,
        service: &str,
        input: Value,
    ) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Fleet {
            fleet_id: String,
        }
        let input: Fleet = serde_json::from_value(input)?;
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let state = self.store.lock().await.read()?;
        let w = Workflows::load(&state)?;
        let a = w
            .associations
            .values()
            .find(|a| a.fleet_id == input.fleet_id)
            .ok_or_else(|| fail(404, "association_not_found"))?;
        let admin = identity.admin && identity.user == a.administrator_mxid;
        if service == "palpo.fleets.install" {
            if !admin {
                return Err(fail(403, "designated_admin_required"));
            }
            if a.state != "approved" {
                return Err(fail(409, "association_not_approved"));
            }
            self.install_association(a, &session.token).await?;
            return Ok(
                json!({"action":Workflows::load(&self.store.lock().await.read()?)?.view(&a.id,&identity.user,now_ms())?}),
            );
        }
        if !admin && !w.may_export_profile(&input.fleet_id, &identity.user) {
            return Err(fail(403, "profile_export_forbidden"));
        }
        let fleet = &state["fleets"][&input.fleet_id];
        if a.state != "approved"
            || fleet["installation"] != "installed"
            || !matches!(
                fleet["state"].as_str(),
                Some("ready" | "pending_connection")
            )
        {
            return Err(fail(409, "profile_unavailable"));
        }
        let e = w
            .authority
            .engagements
            .get(&input.fleet_id)
            .ok_or_else(|| fail(409, "engagement_unavailable"))?;
        if e.delegation_expires_at_ms <= now_ms()
            || matches!(
                e.state,
                palpo_hagency_contract::EngagementState::Suspended
                    | palpo_hagency_contract::EngagementState::Revoked
            )
            || !admin && identity.user != e.owner && identity.user != e.coordinator
        {
            return Err(fail(403, "profile_export_forbidden"));
        }
        // Returned only through Rinx's native export adapter. Never store this in
        // an Inbox payload, notice, general fleet list or script response.
        Ok(
            json!({"schemaVersion":1,"fleetId":a.fleet_id,"serverName":self.matrix.server(),"serverOrigin":self.public_origin,
            "runtimeId":a.intent.runtime_id,"credentialVersion":1,"registration":fleet["registration"],
            "transport":{"mode":"outbound","url":fleet["transport"]["url"],"token":fleet["transport"]["token"],"generation":fleet["transport"]["generation"]},"engagement":e}),
        )
    }
}
fn registration(a: &Association, server: &str, relay: &str) -> Value {
    let escaped = server
        .chars()
        .flat_map(|c| {
            ".*+?^${}()|[]\\"
                .contains(c)
                .then_some('\\')
                .into_iter()
                .chain(std::iter::once(c))
        })
        .collect::<String>();
    json!({"id":a.fleet_id,"url":format!("{relay}/api/relay/v2/{}",a.fleet_id),"as_token":secret(),"hs_token":secret(),
        "sender_localpart":format!("{}_representative",a.fleet_id),"namespaces":{"users":[{"exclusive":true,"regex":format!("^@{}_[a-z0-9_]+:{escaped}$",a.fleet_id)}],"aliases":[],"rooms":[]},
        "rate_limited":true,"receive_ephemeral":false})
}
pub(crate) fn registration_matches(actual: &Value, expected: &Value) -> bool {
    [
        "id",
        "url",
        "as_token",
        "hs_token",
        "sender_localpart",
        "rate_limited",
        "namespaces",
    ]
    .iter()
    .all(|key| actual[*key] == expected[*key])
        && actual["receive_ephemeral"] != true
        && actual["io.element.msc4190"] != true
        && actual["protocols"].as_array().is_none_or(Vec::is_empty)
}
