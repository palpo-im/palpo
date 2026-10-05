//! Recovery of already-approved project rooms, with no new allocation decision.
use palpo_hagency_contract::{CommandId, EngagementState, MatrixUserId, ProjectState};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workflow::{Request, Workflows};
use crate::{Result, digest, fail};

pub(crate) fn allowed(w: &Workflows, id: &str, actor: &MatrixUserId, now: u64) -> bool {
    let Some(action) = w.actions.get(id) else {
        return false;
    };
    let Request::Project(p) = &action.request else {
        return false;
    };
    action.state == "approved"
        && w.authority
            .engagements
            .get(p.server_engagement_id.as_str())
            .is_some_and(|e| {
                e.state == EngagementState::Verified
                    && e.delegation_expires_at_ms > now
                    && (actor == &p.owner || actor == &e.owner || actor == &e.coordinator)
            })
        && w.authority
            .projects
            .get(p.project_id.as_str())
            .is_some_and(|g| {
                g.state == ProjectState::Approved
                    && g.server_engagement_id == p.server_engagement_id
                    && g.revision == p.revision
                    && g.owner == p.owner
            })
}
pub(crate) fn pending(w: &Workflows, id: &str, now: u64) -> bool {
    w.project_retries.values().any(|r| {
        r["actionId"] == id
            && r["state"] == "pending"
            && r["command"]["context"]["expiresAtMs"]
                .as_u64()
                .is_some_and(|t| t > now)
    })
}
pub(crate) fn view(w: &Workflows, id: &str) -> Value {
    let observed = w.project_setups.get(id).cloned().unwrap_or(Value::Null);
    let retry = w
        .project_retries
        .values()
        .filter(|r| r["actionId"] == id)
        .max_by_key(|r| r["sequence"].as_u64().unwrap_or(0));
    match retry {
        Some(r) if r["state"] == "pending" => json!({"state":"pending","reason":null}),
        Some(r) if r["state"] == "refused" => json!({"state":"failed","reason":r["reason"]}),
        _ => observed,
    }
}
pub(crate) fn submit(
    w: &mut Workflows,
    state: &Value,
    tx: &rusqlite::Transaction<'_>,
    input: Value,
    actor: &MatrixUserId,
    now: u64,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Intent {
        id: String,
        command_id: CommandId,
        expected_revision: u64,
    }
    let fingerprint = digest(&json!({"actor":actor,"intent":input}))?;
    let input: Intent = serde_json::from_value(input)?;
    let prior_view = w.view(&input.id, actor, now)?;
    let key = input.command_id.as_str();
    if let Some(old) = w.project_retries.get(key) {
        if old["intentDigest"] != fingerprint {
            return Err(fail(409, "idempotency_conflict"));
        }
        return Ok(prior_view);
    }
    if !allowed(w, &input.id, actor, now) {
        return Err(fail(403, "project_setup_forbidden"));
    }
    let action = &w.actions[&input.id];
    if action.revision != input.expected_revision {
        return Err(fail(409, "request_revision_changed"));
    }
    let Request::Project(p) = &action.request else {
        return Err(fail(400, "project_required"));
    };
    let fleet = &state["fleets"][p.server_engagement_id.as_str()];
    if !crate::views::engagement_available(state, w, p.server_engagement_id.as_str(), now)
        || fleet["capabilities"]["coordinatorProjectSetupV1"] != true
    {
        return Err(fail(409, "project_setup_unavailable"));
    }
    crate::fleet_admin::ensure_active(state, p.server_engagement_id.as_str())?;
    if w.receipts.contains_key(key)
        || w.outbox.contains_key(key)
        || w.agent_controls.contains_key(key)
    {
        return Err(fail(409, "command_id_conflict"));
    }
    if w.project_retries.values().any(|r| {
        r["actionId"] == input.id
            && r["state"] == "pending"
            && r["command"]["context"]["expiresAtMs"]
                .as_u64()
                .is_some_and(|t| t > now)
    }) {
        return Err(fail(409, "project_setup_pending"));
    }
    if w.project_retries.len() >= 10000 {
        return Err(fail(429, "project_setup_history_full"));
    }
    let source = w
        .outbox
        .iter()
        .find(|(_, r)| r["actionId"] == input.id && r["queued"] == true && r["state"] == "applied")
        .map(|(id, _)| id.clone())
        .ok_or_else(|| fail(409, "project_approval_not_delivered"))?;
    let e = &w.authority.engagements[p.server_engagement_id.as_str()];
    let command = json!({"context":{"version":1,"commandId":key,"serverEngagementId":e.id,"registrationGeneration":e.registration_generation,"delegationRevision":e.delegation_revision,"actor":actor,"issuedAtMs":now,"expiresAtMs":now+600000},"projectId":p.project_id,"projectRevision":p.revision,"approvalCommandId":source});
    let payload = json!({"operation":"coordinator_project_setup","command":command});
    crate::outbound::enqueue(
        tx,
        fleet,
        "work",
        "request",
        key,
        &payload,
        Default::default(),
    )?;
    let sequence = w
        .project_retries
        .values()
        .filter(|r| r["actionId"] == input.id)
        .filter_map(|r| r["sequence"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    w.project_retries.insert(key.into(),json!({"actionId":input.id,"sequence":sequence,"intentDigest":fingerprint,"commandDigest":digest(&payload)?,"command":command,"transportGeneration":fleet["transport"]["generation"],"state":"pending"}));
    let action = w.actions.get_mut(&input.id).unwrap();
    action.revision += 1;
    action.updated_at = now;
    w.notify(&input.id, now)?;
    w.view(&input.id, actor, now)
}
pub(crate) fn observation(
    w: &mut Workflows,
    fleet: &Value,
    update: &Value,
    now: u64,
) -> Result<()> {
    let body = &update["payload"];
    let source = body["approvalCommandId"]
        .as_str()
        .and_then(|id| w.outbox.get(id))
        .ok_or_else(|| fail(409, "project_approval_missing"))?;
    let action_id = source["actionId"]
        .as_str()
        .ok_or_else(|| fail(409, "action_not_found"))?
        .to_owned();
    let action = w
        .actions
        .get(&action_id)
        .ok_or_else(|| fail(409, "action_not_found"))?;
    let Request::Project(p) = &action.request else {
        return Err(fail(409, "project_required"));
    };
    let attempt = body["attemptId"]
        .as_str()
        .ok_or_else(|| fail(400, "invalid_project_setup"))?;
    let retry = w.project_retries.get(attempt);
    let max = palpo_hagency_contract::MAX_EXACT_JSON_INTEGER;
    if source["queued"] != true
        || source["transportGeneration"] != fleet["transport"]["generation"]
        || source["serverEngagementId"] != fleet["id"]
        || p.server_engagement_id.as_str() != fleet["id"].as_str().unwrap_or_default()
        || body["projectId"] != p.project_id.as_str()
        || body["projectRevision"] != json!(p.revision)
        || update["id"] != format!("project_setup_{}", p.project_id.as_str())
        || action.state != "approved"
        || body["revision"].as_u64().is_none_or(|r| r == 0 || r > max)
        || body["observedAtMs"]
            .as_u64()
            .is_none_or(|t| t > now.saturating_add(5000))
        || !matches!(body["state"].as_str(), Some("pending" | "failed" | "ready"))
        || (body["state"] == "failed") != body["reason"].is_string()
        || body["reason"].as_str().is_some_and(|r| {
            !matches!(
                r,
                "project_membership_pending"
                    | "private_membership_pending"
                    | "project_room_unreadable"
                    | "private_room_unreadable"
                    | "room_authority_changed"
                    | "authority_changed"
                    | "setup_unavailable"
            )
        })
        || !(body["attemptId"] == body["approvalCommandId"]
            || retry.is_some_and(|r| {
                r["actionId"] == action_id
                    && r["transportGeneration"] == fleet["transport"]["generation"]
                    && r["command"]["approvalCommandId"] == body["approvalCommandId"]
            }))
    {
        return Err(fail(409, "project_setup_binding_conflict"));
    }
    if let Some(old) = w.project_setups.get(&action_id) {
        if body["revision"].as_u64() < old["revision"].as_u64() {
            return Ok(());
        }
        if body["revision"] == old["revision"] {
            if old != body {
                return Err(fail(409, "project_setup_revision_conflict"));
            }
            return Ok(());
        }
    }
    w.project_setups.insert(action_id.clone(), body.clone());
    if let Some(retry) = w.project_retries.get_mut(attempt) {
        retry["state"] = body["state"].clone();
    }
    let action = w.actions.get_mut(&action_id).unwrap();
    action.revision += 1;
    action.updated_at = now;
    w.notify(&action_id, now)
}
pub(crate) fn refusal(w: &mut Workflows, fleet: &Value, update: &Value, now: u64) -> Result<bool> {
    let r = &update["payload"];
    let Some(key) = r["commandId"].as_str() else {
        return Ok(false);
    };
    let Some(row) = w.project_retries.get_mut(key) else {
        return Ok(false);
    };
    if r["state"] != "refused"
        || r["operation"] != "project_setup"
        || r["projectId"] != row["command"]["projectId"]
        || r["commandDigest"] != row["commandDigest"]
        || row["command"]["context"]["serverEngagementId"] != fleet["id"]
        || row["transportGeneration"] != fleet["transport"]["generation"]
        || update["id"] != format!("command_{key}")
        || !matches!(
            r["reason"].as_str(),
            Some(
                "authority_changed"
                    | "invalid_request"
                    | "project_unavailable"
                    | "resource_unavailable"
                    | "request_conflict"
                    | "insufficient_capacity"
            )
        )
    {
        return Err(fail(409, "project_setup_receipt_conflict"));
    }
    if let Some(previous) = row.get("result") {
        if previous != r {
            return Err(fail(409, "project_setup_receipt_conflict"));
        }
        return Ok(true);
    }
    row["result"] = r.clone();
    row["state"] = json!("refused");
    row["reason"] = r["reason"].clone();
    let id = row["actionId"].as_str().unwrap().to_owned();
    let action = w.actions.get_mut(&id).unwrap();
    action.revision += 1;
    action.updated_at = now;
    w.notify(&id, now)?;
    Ok(true)
}
