//! Scoped runtime controls. A transport ACK is never a cleanup receipt.
use crate::{
    Result, digest, fail,
    workflow::{Request, Workflows},
};
use palpo_hagency_contract::{CommandId, EngagementState, MatrixUserId};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) fn allowed(
    w: &Workflows,
    state: &Value,
    id: &str,
    actor: &MatrixUserId,
    now: u64,
) -> bool {
    let Some(action) = w.actions.get(id) else {
        return false;
    };
    let Request::Agent(agent) = &action.request else {
        return false;
    };
    let Some(authority) = w
        .authority
        .engagements
        .get(agent.server_engagement_id.as_str())
    else {
        return false;
    };
    let fleet = &state["fleets"][agent.server_engagement_id.as_str()];
    action.state == "approved"
        && fleet["capabilities"]["coordinatorAgentControlV1"] == true
        && fleet["installation"] == "installed"
        && fleet["state"] == "ready"
        && (actor == &agent.project_owner
            || actor == &authority.owner
            || actor == &authority.coordinator && authority.delegation_expires_at_ms > now)
        && w.observations.get(id).is_some_and(|o| {
            o["engagementId"].as_str().is_some()
                && o["generation"] == fleet["transport"]["generation"]
        })
}

pub(crate) fn latest<'a>(w: &'a Workflows, id: &str) -> Option<&'a Value> {
    w.agent_controls
        .values()
        .filter(|r| r["actionId"] == id)
        .max_by_key(|r| r["sequence"].as_u64().unwrap_or_default())
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
        #[serde(default)]
        kind: Option<String>,
        agent_action_id: String,
        command_id: CommandId,
        operation: String,
        #[serde(default)]
        display_name: Option<String>,
    }
    let fingerprint = digest(&json!({"actor":actor,"intent":input}))?;
    let input: Intent = serde_json::from_value(input)?;
    if input.kind.as_deref().is_some_and(|k| k != "agent_control") {
        return Err(fail(400, "invalid_control_kind"));
    }
    if (input.operation == "rename") != input.display_name.is_some()
        || input.display_name.as_ref().is_some_and(|name| {
            name.trim() != name
                || name.is_empty()
                || name.chars().count() > 128
                || name.chars().any(char::is_control)
        })
    {
        return Err(fail(400, "invalid_display_name"));
    }
    if !allowed(w, state, &input.agent_action_id, actor, now) {
        return Err(fail(403, "agent_control_forbidden"));
    }
    let key = input.command_id.as_str();
    if let Some(previous) = w.agent_controls.get(key) {
        if previous["intentDigest"] != fingerprint {
            return Err(fail(409, "idempotency_conflict"));
        }
        return w.view(&input.agent_action_id, actor, now);
    }
    if w.receipts.contains_key(key)
        || w.outbox.contains_key(key)
        || w.project_retries.contains_key(key)
    {
        return Err(fail(409, "command_id_conflict"));
    }
    let action = &w.actions[&input.agent_action_id];
    let Request::Agent(agent) = &action.request else {
        return Err(fail(400, "agent_required"));
    };
    let authority = &w.authority.engagements[agent.server_engagement_id.as_str()];
    let observation = &w.observations[&action.id];
    let prior = latest(w, &action.id);
    if prior.is_some_and(|r| {
        r["state"] == "pending"
            || r["execution"] == "retiring"
            || r["execution"] == "inspection_required"
    }) {
        return Err(fail(409, "agent_control_pending"));
    }
    match input.operation.as_str() {
        "retire" if matches!(observation["state"].as_str(), Some("pending" | "active")) => {}
        "stop" if observation["state"] == "active" => {}
        "start"
            if observation["state"] == "active"
                && authority.state == EngagementState::Verified
                && authority.delegation_expires_at_ms > now => {}
        "rename"
            if observation["state"] == "active"
                && authority.state == EngagementState::Verified
                && authority.delegation_expires_at_ms > now
                && state["fleets"][authority.id.as_str()]["capabilities"]["coordinatorAgentProfileV1"]
                    == true => {}
        "retry_cleanup" if observation["lifecycle"]["cleanupEffect"] == "failed" => {}
        _ => return Err(fail(409, "agent_control_unavailable")),
    }
    if w.agent_controls.len() >= 10000 {
        return Err(fail(429, "agent_control_history_full"));
    }
    let sequence = prior.and_then(|r| r["sequence"].as_u64()).unwrap_or(0) + 1;
    let mut command = json!({"context":{"version":1,"commandId":input.command_id,"serverEngagementId":authority.id,
        "registrationGeneration":authority.registration_generation,"delegationRevision":authority.delegation_revision,"actor":actor,
        "issuedAtMs":now,"expiresAtMs":now+600000},"agentAllocationId":observation["engagementId"],
        "projectId":agent.project_id,"projectRevision":agent.project_revision,"resourceAllocationId":agent.resource_allocation_id,"operation":input.operation});
    if let Some(name) = input.display_name {
        command["displayName"] = json!(name);
    }
    let payload = json!({"operation":"coordinator_agent_control","command":command});
    let fleet = &state["fleets"][authority.id.as_str()];
    crate::outbound::enqueue(
        tx,
        fleet,
        "work",
        "request",
        key,
        &payload,
        Default::default(),
    )?;
    w.agent_controls.insert(key.into(),json!({"actionId":input.agent_action_id,"intentDigest":fingerprint,"commandDigest":digest(&payload)?,"command":command,
        "serverEngagementId":authority.id,"transportGeneration":fleet["transport"]["generation"],"sequence":sequence,
        "state":"pending","execution":"control_pending","createdAt":now}));
    let action = w.actions.get_mut(&input.agent_action_id).unwrap();
    action.revision += 1;
    action.updated_at = now;
    w.notify(&input.agent_action_id, now)?;
    w.view(&input.agent_action_id, actor, now)
}

pub(crate) fn receipt(w: &mut Workflows, fleet: &Value, update: &Value, now: u64) -> Result<bool> {
    let receipt = &update["payload"];
    let key = receipt["commandId"]
        .as_str()
        .ok_or_else(|| fail(400, "invalid_receipt"))?;
    let Some(record) = w.agent_controls.get_mut(key) else {
        return Ok(false);
    };
    if receipt["state"] == "refused"
        && !matches!(
            receipt["reason"].as_str(),
            Some(
                "insufficient_capacity"
                    | "authority_changed"
                    | "invalid_request"
                    | "project_unavailable"
                    | "resource_unavailable"
                    | "request_conflict"
            )
        )
    {
        return Err(fail(409, "invalid_refusal"));
    }
    if update["id"] != format!("command_{key}")
        || record["serverEngagementId"] != fleet["id"]
        || record["transportGeneration"] != fleet["transport"]["generation"]
        || receipt["commandDigest"] != record["commandDigest"]
        || receipt["agentId"] != record["command"]["agentAllocationId"]
        || receipt["operation"] != record["command"]["operation"]
        || !matches!(receipt["state"].as_str(), Some("applied" | "refused"))
        || !record["result"].is_null() && record["result"] != *receipt
    {
        return Err(fail(409, "control_receipt_conflict"));
    }
    record["state"] = receipt["state"].clone();
    record["result"] = receipt.clone();
    refresh(w, now)?;
    Ok(true)
}

pub(crate) fn refresh(w: &mut Workflows, now: u64) -> Result<()> {
    let mut changed = Vec::new();
    for row in w.agent_controls.values_mut() {
        let id = row["actionId"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_agent_control"))?
            .to_owned();
        let status = w.observations.get(&id).cloned().unwrap_or(Value::Null);
        let execution = if row["state"] == "refused" {
            "control_refused"
        } else if row["state"] == "pending" {
            "control_pending"
        } else if matches!(
            row["command"]["operation"].as_str(),
            Some("retire" | "retry_cleanup")
        ) {
            match status["lifecycle"]["cleanupEffect"].as_str() {
                Some("failed") => "cleanup_failed",
                Some("uncertain" | "started") if status["lifecycle"]["cleanup"] == "uncertain" => {
                    "inspection_required"
                }
                _ if status["lifecycle"]["runtimeState"] == "revoked"
                    && matches!(
                        status["lifecycle"]["cleanup"].as_str(),
                        Some("complete" | "not_required")
                    ) =>
                {
                    "retired"
                }
                _ => "retiring",
            }
        } else if row["command"]["operation"] == "rename" {
            let profile = &status["lifecycle"]["matrixProfile"];
            if profile["desiredName"] != row["command"]["displayName"] {
                "rename_pending"
            } else if profile["state"] == "failed" {
                "rename_failed"
            } else if profile["state"] == "verified"
                && profile["observedName"] == row["command"]["displayName"]
            {
                "control_applied"
            } else {
                "rename_pending"
            }
        } else {
            "control_applied"
        };
        if row["execution"] != execution {
            row["execution"] = json!(execution);
            changed.push(id);
        }
    }
    changed.sort();
    changed.dedup();
    for id in changed {
        if let Some(action) = w.actions.get_mut(&id) {
            action.revision += 1;
            action.updated_at = now;
        }
        w.notify(&id, now)?;
    }
    Ok(())
}
