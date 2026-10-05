//! Authenticated Hagency observations. Custody ACK, execution, Matrix readiness,
//! and token metering are distinct facts; receipt time never refreshes old data.
use chrono::{DateTime, SecondsFormat};
use palpo_hagency_contract::canonical::transport_digest;
use palpo_hagency_contract::{MatrixUserId, ProjectGrant, ProjectState, ServerName};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::workflow::{Request, Resource, Workflows};
use crate::{Result, digest, fail};

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
        .ok_or_else(|| fail(400, "invalid_update_field"))
}
fn iso(now: u64) -> Result<String> {
    i64::try_from(now)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| fail(400, "invalid_clock"))
}
fn millis(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .and_then(|d| d.timestamp_millis().try_into().ok())
}
pub fn status_current(observation: &Value, now: u64) -> bool {
    let Some(observed) = millis(&observation["observedAt"]) else {
        return false;
    };
    let Some(received) = observation["receivedAtMs"].as_u64() else {
        return false;
    };
    observed <= received.saturating_add(5000) && now.saturating_sub(observed.min(received)) < 90000
}

/// Validate before any network call, and repeat under the committing writer.
/// Identical sequence retries do not require another Matrix observation.
pub fn sequence(fleet: &Value, input: &Value) -> Result<bool> {
    if input["v"] != 2
        || input["generation"] != fleet["transport"]["generation"]
        || input["heartbeat"] != true
        || input["sequence"]
            .as_u64()
            .is_none_or(|n| n == 0 || n > 9_007_199_254_740_991)
        || [
            ("statuses", 200),
            ("probeReceipts", 10),
            ("coordinatorUpdates", 100),
        ]
        .iter()
        .any(|(key, max)| {
            input
                .get(key)
                .is_some_and(|v| v.as_array().is_none_or(|a| a.len() > *max))
        })
    {
        return Err(fail(400, "invalid_update"));
    }
    let n = input["sequence"].as_u64().unwrap();
    let last = fleet["transport"]["sequence"].as_u64().unwrap_or(0);
    if n < last {
        return Err(fail(409, "stale_sequence"));
    }
    if n == last {
        if fleet["transport"]["updateDigest"] != transport_digest(input)? {
            return Err(fail(409, "sequence_conflict"));
        }
        return Ok(false);
    }
    Ok(true)
}

fn capabilities(fleet: &mut Value, input: &Value, server: &ServerName, now: u64) -> Result<()> {
    if input["v"] != 1
        || input["fleetId"] != fleet["id"]
        || input["serverName"] != server.as_str()
        || input["representativeMxid"] != fleet["representativeMxid"]
    {
        return Err(fail(409, "provider_identity_mismatch"));
    }
    let bot: MatrixUserId = text(input, "approvalBotMxid")?.to_owned().try_into()?;
    // Identity is derived from the installed namespace, never from an arbitrary
    // address supplied by a provider sharing the same Matrix hostname.
    let expected = format!("@{}_approval:{}", text(fleet, "id")?, server.as_str());
    if bot.as_str() != expected {
        return Err(fail(409, "provider_capability_missing"));
    }
    let offers = input["offers"]
        .as_array()
        .filter(|o| o.len() <= 100)
        .ok_or_else(|| fail(400, "invalid_capabilities"))?;
    let mut clean = Vec::new();
    for offer in offers {
        if offer["published"] == false {
            continue;
        }
        let role = text(offer, "role")?;
        if role.len() > 80 {
            return Err(fail(400, "invalid_capabilities"));
        }
        let mut resources = Vec::new();
        if let Some(list) = offer.get("resources") {
            for resource in list
                .as_array()
                .filter(|r| r.len() <= 200)
                .ok_or_else(|| fail(400, "invalid_capabilities"))?
            {
                let id = text(resource, "id")?;
                if id.len() != 33
                    || !id.starts_with("resource_")
                    || !id[9..]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(fail(400, "invalid_capabilities"));
                }
                resources.push(json!({"id":id,"name":text(resource,"name")?,"framework":text(resource,"framework")?,"model":text(resource,"model")?,
                    "reasoning":resource["reasoning"].as_str().map(|s| s.chars().take(64).collect::<String>())}));
            }
        }
        clean.push(json!({"role":role,"resources":resources,"description":offer["description"].as_str().map(|s| s.chars().take(500).collect::<String>())}));
    }
    fleet["capabilities"] = json!({"v":1,"fleetId":fleet["id"],"serverName":server,"representativeMxid":fleet["representativeMxid"],
        "approvalBotMxid":bot,"offers":clean,"coordinatorApprovalV1":input["coordinatorApprovalV1"]==true,
        "coordinatorAgentControlV1":input["coordinatorAgentControlV1"]==true,"observedAt":iso(now)?});
    fleet["capabilityRead"] = json!({"state":"current","observedAt":iso(now)?});
    Ok(())
}

fn probe(tx: &Transaction<'_>, fleet: &Value, receipt: &Value) -> Result<()> {
    let p = &fleet["probe"];
    if !receipt.is_object()
        || receipt["received"] != true
        || receipt["fleetId"] != fleet["id"]
        || p["eventId"].as_str().is_none()
        || p["challenge"].as_str().is_none()
        || receipt["sourceRoomId"] != p["roomId"]
        || receipt["sourceEventId"] != p["eventId"]
        || p["matrixEventId"] != p["eventId"]
        || receipt["challenge"] != p["challenge"]
    {
        return Err(fail(409, "probe_binding_conflict"));
    }
    tx.execute_batch(crate::outbound::SCHEMA)?;
    let ack: Option<Option<u64>> = tx.query_row("SELECT acked FROM fleet_delivery WHERE fleet=?1 AND generation=?2 AND lane='matrix' AND id=?3",
        params![fleet["id"].as_str(),fleet["transport"]["generation"].as_u64(),p["matrixTransactionId"].as_str()], |r|r.get(0)).optional()?;
    if ack.flatten().is_none() {
        return Err(fail(409, "matrix_receipt_pending"));
    }
    Ok(())
}

fn coordinator(
    workflows: &mut Workflows,
    fleet: &Value,
    updates: &[Value],
    server: &ServerName,
    now: u64,
) -> Result<()> {
    let id = text(fleet, "id")?;
    let mut snapshot = workflows.authority.clone();
    let mut changed_delegation = false;
    // Only this engagement's authenticated runtime can publish an owner change.
    // It cannot advance registration, invent a probe or grant itself capability.
    for update in updates
        .iter()
        .filter(|u| u["payload"]["kind"] == "engagement")
    {
        let body = &update["payload"];
        let next: palpo_hagency_contract::ServerEngagement =
            serde_json::from_value(body["engagement"].clone())?;
        let previous = snapshot
            .engagements
            .get(id)
            .ok_or_else(|| fail(409, "engagement_not_registered"))?;
        let exports: Vec<MatrixUserId> = serde_json::from_value(body["exportMxids"].clone())?;
        if update["id"] != "engagement_authority"
            || update["digest"] != digest(body)?
            || next.id.as_str() != id
            || next.owner != previous.owner
            || next.server != previous.server
            || next.registration_generation != previous.registration_generation
            || next.coordinator_approval_v1 != previous.coordinator_approval_v1
            || next.delegation_revision < previous.delegation_revision
            || body["registrationGeneration"] != json!(next.registration_generation)
            || body["delegationRevision"] != json!(next.delegation_revision)
            || body["observedAtMs"]
                .as_u64()
                .is_none_or(|n| n > now.saturating_add(5000))
            || exports.len() > 2
            || exports
                .iter()
                .any(|u| u != &next.owner && u != &next.coordinator)
            || exports
                .iter()
                .map(|u| u.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != exports.len()
            || !next.coordinator.belongs_to(server)
            || !matches!(
                next.state,
                palpo_hagency_contract::EngagementState::Verified
                    | palpo_hagency_contract::EngagementState::Suspended
                    | palpo_hagency_contract::EngagementState::Revoked
            )
            || next.state == palpo_hagency_contract::EngagementState::Verified
                && !matches!(
                    previous.state,
                    palpo_hagency_contract::EngagementState::Verified
                        | palpo_hagency_contract::EngagementState::Suspended
                )
        {
            return Err(fail(409, "delegation_binding_conflict"));
        }
        if next.delegation_revision == previous.delegation_revision
            && (next != *previous || workflows.engagement_exports.get(id) != Some(&exports))
        {
            return Err(fail(409, "delegation_revision_conflict"));
        }
        changed_delegation |= next != *previous;
        snapshot.engagements.insert(id.into(), next);
        workflows.engagement_exports.insert(id.into(), exports);
    }
    let authority = snapshot
        .engagements
        .get(id)
        .ok_or_else(|| fail(409, "engagement_not_registered"))?;
    // Appservice registration and outbound credential rotations have separate
    // generations. Legacy registrations predate this field and start at one.
    if u64::from(authority.registration_generation)
        != fleet
            .get("registrationGeneration")
            .map_or(Some(1), Value::as_u64)
            .unwrap_or(0)
    {
        return Err(fail(409, "generation_conflict"));
    }
    // Apply parent grants first even if the network batch orders receipts first.
    for update in updates {
        if !update["payload"].is_object() || update["digest"] != digest(&update["payload"])? {
            return Err(fail(409, "projection_digest_conflict"));
        }
        let body = &update["payload"];
        if body["registrationGeneration"] != json!(authority.registration_generation)
            || body["delegationRevision"]
                .as_u64()
                .is_none_or(|r| r == 0 || r > u64::from(authority.delegation_revision))
        {
            return Err(fail(409, "projection_authority_changed"));
        }
        match body["kind"].as_str() {
            Some("resource") => {
                let grant: Resource = serde_json::from_value(body["resource"].clone())?;
                if grant.server_engagement_id.as_str() != id
                    || update["id"] != format!("resource_{}", grant.id.as_str())
                {
                    return Err(fail(409, "projection_binding_conflict"));
                }
                if let Some(previous) = workflows.resource_details.get(grant.id.as_str())
                    && (previous["resourceId"] != body["resourceId"]
                        || previous["period"] != body["period"]
                        || previous["periodKey"] != body["periodKey"])
                {
                    return Err(fail(409, "resource_binding_conflict"));
                }
                workflows.resource_details.insert(grant.id.as_str().into(),
                    json!({"resourceId":body["resourceId"],"period":body["period"],"periodKey":body["periodKey"]}));
                snapshot.resources.insert(grant.id.as_str().into(), grant);
            }
            Some("project" | "receipt" | "engagement") => {}
            _ => return Err(fail(400, "invalid_projection")),
        }
    }
    for update in updates.iter().filter(|u| u["payload"]["kind"] == "project") {
        let grant: ProjectGrant = serde_json::from_value(update["payload"]["project"].clone())?;
        if grant.server_engagement_id.as_str() != id
            || update["id"] != format!("project_{}", grant.project_id.as_str())
        {
            return Err(fail(409, "projection_binding_conflict"));
        }
        // Only a previously approved, delivered decision may create a project.
        let authorized = workflows.outbox.values().any(|r| r["serverEngagementId"]==id && r["queued"]==true && r["command"]["request"]["projectId"]==grant.project_id.as_str()
            && workflows.actions.get(r["actionId"].as_str().unwrap_or_default()).is_some_and(|a| matches!(&a.request, Request::Project(p)
                if p.owner==grant.owner && p.revision==grant.revision && p.resource_allocations==grant.resource_allocations)));
        if !authorized {
            return Err(fail(409, "project_decision_missing"));
        }
        snapshot
            .projects
            .insert(grant.project_id.as_str().into(), grant);
    }
    workflows.import_authority(snapshot, server)?;
    if changed_delegation {
        let actions = workflows
            .actions
            .values()
            .filter(|a| a.request.engagement().as_str() == id && a.state == "requested")
            .map(|a| a.id.clone())
            .collect::<Vec<_>>();
        for action in actions {
            let row = workflows
                .actions
                .get_mut(&action)
                .ok_or_else(|| fail(409, "action_not_found"))?;
            row.revision += 1;
            row.updated_at = now;
            workflows.notify(&action, now)?;
        }
        let association = workflows
            .associations
            .values_mut()
            .find(|a| a.fleet_id == id)
            .map(|action| {
                action.execution = serde_json::to_value(workflows.authority.engagements[id].state)?
                    .as_str()
                    .unwrap_or("unknown")
                    .into();
                action.revision += 1;
                action.updated_at = now;
                Ok::<_, crate::Error>(action.id.clone())
            })
            .transpose()?;
        if let Some(action) = association {
            workflows.notify_association(&action, now)?;
        }
    }
    for update in updates.iter().filter(|u| u["payload"]["kind"] == "receipt") {
        if crate::lifecycle::receipt(workflows, fleet, update, now)? {
            continue;
        }
        let receipt = &update["payload"];
        let command_id = text(receipt, "commandId")?;
        let record = workflows
            .outbox
            .get_mut(command_id)
            .ok_or_else(|| fail(409, "command_not_found"))?;
        if update["id"] != format!("command_{command_id}")
            || record["serverEngagementId"] != id
            || record["queued"] != true
            || record["transportGeneration"] != fleet["transport"]["generation"]
            || !matches!(receipt["state"].as_str(), Some("applied" | "refused"))
        {
            return Err(fail(409, "receipt_binding_conflict"));
        }
        let action = workflows
            .actions
            .get(record["actionId"].as_str().unwrap_or_default())
            .ok_or_else(|| fail(409, "action_not_found"))?;
        let expected = match &action.request {
            Request::Project(request) => {
                if receipt["projectId"] != request.project_id.as_str()
                    || !receipt["agentId"].is_null()
                {
                    return Err(fail(409, "receipt_binding_conflict"));
                }
                let key: String = request.definition_digest.clone().into();
                digest(
                    &json!({"operation":"coordinator_project_approval","command":record["command"],"definition":workflows.definitions[&key]}),
                )?
            }
            Request::Agent(_) => {
                if receipt["state"] == "applied" && text(receipt, "agentId")?.len() > 128
                    || receipt["state"] == "refused" && !receipt["agentId"].is_null()
                    || !receipt["projectId"].is_null()
                {
                    return Err(fail(409, "receipt_binding_conflict"));
                }
                digest(
                    &json!({"operation":"coordinator_agent_approval","command":record["command"]}),
                )?
            }
            Request::TokenTopUp(request) => {
                if receipt["agentId"] != request.agent_allocation_id.as_str()
                    || !receipt["projectId"].is_null()
                {
                    return Err(fail(409, "receipt_binding_conflict"));
                }
                digest(
                    &json!({"operation":"coordinator_token_top_up","command":record["command"]}),
                )?
            }
        };
        if receipt["commandDigest"] != expected
            || (!record["result"].is_null() && record["result"] != *receipt)
        {
            return Err(fail(409, "receipt_digest_conflict"));
        }
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
        record["state"] = receipt["state"].clone();
        record["result"] = receipt.clone();
    }
    refresh_executions(workflows, now)
}

fn execution(workflows: &mut Workflows, id: &str, next: &str, now: u64) -> Result<()> {
    let action = workflows
        .actions
        .get_mut(id)
        .ok_or_else(|| fail(409, "action_not_found"))?;
    if action.execution != next {
        action.execution = next.into();
        action.updated_at = now;
        action.revision += 1;
        workflows.notify(id, now)?;
    }
    Ok(())
}
fn refresh_executions(workflows: &mut Workflows, now: u64) -> Result<()> {
    let mut changes = Vec::new();
    for record in workflows
        .outbox
        .values()
        .filter(|r| matches!(r["state"].as_str(), Some("applied" | "refused")))
    {
        let id = record["actionId"]
            .as_str()
            .ok_or_else(|| fail(409, "action_not_found"))?;
        let action = &workflows.actions[id];
        if record["state"] == "refused" {
            changes.push((id.to_owned(), "allocation_refused"));
            continue;
        }
        let next = match &action.request {
            Request::Project(p) => {
                if workflows
                    .authority
                    .projects
                    .get(p.project_id.as_str())
                    .is_some_and(|p| p.state == ProjectState::Ready)
                {
                    "ready"
                } else {
                    "provisioning"
                }
            }
            Request::Agent(_) => {
                let status = &workflows
                    .observations
                    .get(id)
                    .cloned()
                    .unwrap_or(Value::Null);
                if status["engagementId"] != record["result"]["agentId"] && !status.is_null() {
                    return Err(fail(409, "agent_binding_conflict"));
                }
                if status["ready"] == true && status_current(status, now) {
                    "ready"
                } else if status["ready"] == true {
                    "unknown"
                } else if matches!(status["state"].as_str(), Some("ended" | "rejected")) {
                    "ended"
                } else {
                    "provisioning"
                }
            }
            Request::TokenTopUp(_) => "done",
        };
        changes.push((id.to_owned(), next));
    }
    for (id, next) in changes {
        execution(workflows, &id, next, now)?;
    }
    crate::lifecycle::refresh(workflows, now)
}

fn status(
    workflows: &mut Workflows,
    state: &mut Value,
    fleet: &Value,
    raw: &Value,
    now: u64,
) -> Result<()> {
    if raw["v"] != 1
        || raw["fleetId"] != fleet["id"]
        || !matches!(
            raw["state"].as_str(),
            Some("pending" | "active" | "ended" | "rejected")
        )
    {
        return Err(fail(400, "invalid_status"));
    }
    let request_id = text(raw, "requestId")?;
    let action = workflows
        .actions
        .values()
        .find(|a| {
            matches!(&a.request, Request::Agent(_))
                && a.request.engagement().as_str() == fleet["id"].as_str().unwrap_or_default()
                && a.request.id().as_str() == request_id
        })
        .cloned();
    let legacy_id = format!("{}:{request_id}", text(fleet, "id")?);
    let definition = if let Some(a) = &action {
        let key: String = a.request.definition_digest().clone().into();
        workflows
            .definitions
            .get(&key)
            .cloned()
            .ok_or_else(|| fail(409, "request_definition_missing"))?
    } else {
        state["requests"][&legacy_id]["payload"]
            .as_object()
            .map(|o| Value::Object(o.clone()))
            .ok_or_else(|| fail(409, "unknown_request"))?
    };
    for field in [
        "requestId",
        "fleetId",
        "targetProjectId",
        "targetRoomId",
        "sourceRoomId",
        "sourceEventId",
        "role",
        "requestedTokens",
        "agentDefinition",
    ] {
        if raw[field] != definition[field] {
            return Err(fail(409, "request_binding_conflict"));
        }
    }
    let mut clean = serde_json::Map::new();
    for field in [
        "v",
        "fleetId",
        "requestId",
        "engagementId",
        "state",
        "targetProjectId",
        "targetRoomId",
        "sourceRoomId",
        "sourceEventId",
        "role",
        "agentDefinition",
        "requestedTokens",
        "allocatedTokens",
        "agentMxid",
        "bound",
        "ready",
        "observedAt",
        "consumedTokens",
        "usageObservedAt",
        "usageObservedAtMs",
        "usageEvidence",
        "usageComplete",
        "quotaPaused",
        "lifecycle",
    ] {
        if let Some(value) = raw.get(field) {
            clean.insert(field.into(), value.clone());
        }
    }
    let mut clean = Value::Object(clean);
    if !raw["allocatedTokens"].is_null() {
        let _: palpo_hagency_contract::Tokens =
            serde_json::from_value(raw["allocatedTokens"].clone())?;
    }
    if !raw["consumedTokens"].is_null() {
        let _: palpo_hagency_contract::Tokens =
            serde_json::from_value(raw["consumedTokens"].clone())?;
    }
    if let Some(agent) = raw["agentMxid"].as_str() {
        let expected = format!(
            "@{}_{}:{}",
            text(fleet, "id")?,
            text(raw, "engagementId")?,
            fleet["representativeMxid"]
                .as_str()
                .and_then(|s| s.split_once(':').map(|(_, s)| s))
                .unwrap_or_default()
        );
        if agent != expected {
            return Err(fail(409, "agent_namespace_conflict"));
        }
    }
    if raw["ready"] == true
        && (raw["state"] != "active"
            || raw["bound"] != true
            || raw["agentMxid"].as_str().is_none()
            || raw["fulfillment"]["phase"] != "complete"
            || raw["fulfillment"]["incomplete"] == true)
    {
        return Err(fail(409, "readiness_conflict"));
    }
    clean["receivedAtMs"] = json!(now);
    clean["generation"] = fleet["transport"]["generation"].clone();
    if let Some(a) = action {
        if a.state != "approved" {
            return Err(fail(409, "agent_decision_missing"));
        }
        workflows.observations.insert(a.id, clean);
    } else {
        let record = &mut state["requests"][&legacy_id];
        record["provider"] = clean;
        record["state"] = raw["state"].clone();
        record["usable"] = json!(false);
        record["statusVerified"] = json!(true);
        record["observedAt"] = json!(iso(now)?);
        record["outboundStatus"] = json!({"generation":fleet["transport"]["generation"],"observedAt":raw["observedAt"],"receivedAt":iso(now)?});
        if !record["retirement"].is_null() {
            record["state"] = json!("ended");
            record["provider"]["state"] = json!("ended");
            record["provider"]["ready"] = json!(false);
            record["provider"]["bound"] = json!(false);
        }
    }
    Ok(())
}

pub fn apply(
    state: &mut Value,
    tx: &Transaction<'_>,
    mut fleet: Value,
    input: &Value,
    room: Option<&Value>,
    server: &ServerName,
    now: u64,
) -> Result<Value> {
    if !sequence(&fleet, input)? {
        return Ok(json!({"ok":true}));
    }
    if let Some(c) = input.get("capabilities") {
        capabilities(&mut fleet, c, server, now)?;
    }
    let receipts = input["probeReceipts"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for r in &receipts {
        probe(tx, &fleet, r)?;
    }
    if !receipts.is_empty() {
        let events = room
            .and_then(Value::as_array)
            .ok_or_else(|| fail(409, "reception_membership_pending"))?;
        for member in [&fleet["ownerMxid"], &fleet["representativeMxid"]] {
            if !events.iter().any(|e| {
                e["type"] == "m.room.member"
                    && e["state_key"] == *member
                    && e["content"]["membership"] == "join"
            }) {
                return Err(fail(409, "reception_membership_pending"));
            }
        }
        fleet["probe"]["completedAt"] = json!(iso(now)?);
        fleet["connection"] = json!({"verifiedAt":iso(now)?,"generation":fleet["transport"]["generation"],"sourceRoomId":fleet["probe"]["roomId"],"sourceEventId":fleet["probe"]["eventId"],"challenge":fleet["probe"]["challenge"]});
        fleet["state"] = json!("ready");
        fleet["lastError"] = Value::Null;
    }
    let mut workflows = Workflows::load(state)?;
    if !receipts.is_empty() {
        let id = text(&fleet, "id")?;
        if let Some(action_id) = workflows
            .associations
            .values()
            .find(|a| a.fleet_id == id && a.state == "approved")
            .map(|a| a.id.clone())
        {
            let e = workflows
                .authority
                .engagements
                .get_mut(id)
                .ok_or_else(|| fail(409, "engagement_not_registered"))?;
            if u64::from(e.registration_generation)
                != fleet["registrationGeneration"].as_u64().unwrap_or(0)
                || matches!(
                    e.state,
                    palpo_hagency_contract::EngagementState::Suspended
                        | palpo_hagency_contract::EngagementState::Revoked
                )
            {
                return Err(fail(409, "engagement_unavailable"));
            }
            e.state = palpo_hagency_contract::EngagementState::Verified;
            e.coordinator_approval_v1 = fleet["capabilities"]["coordinatorApprovalV1"] == true;
            let a = workflows
                .associations
                .get_mut(&action_id)
                .ok_or_else(|| fail(409, "action_not_found"))?;
            if a.execution != "verified" {
                a.execution = "verified".into();
                a.revision += 1;
                a.updated_at = now;
                workflows.notify_association(&action_id, now)?;
            }
        }
    }
    if let Some(updates) = input["coordinatorUpdates"]
        .as_array()
        .filter(|v| !v.is_empty())
    {
        coordinator(&mut workflows, &fleet, updates, server, now)?;
    }
    for raw in input["statuses"].as_array().into_iter().flatten() {
        status(&mut workflows, state, &fleet, raw, now)?;
    }
    refresh_executions(&mut workflows, now)?;
    workflows.save(state)?;
    fleet["transport"]["sequence"] = input["sequence"].clone();
    fleet["transport"]["updateDigest"] = json!(transport_digest(input)?);
    fleet["transport"]["lastSeenAt"] = json!(iso(now)?);
    let id = text(&fleet, "id")?.to_owned();
    state["fleets"][id] = fleet;
    Ok(json!({"ok":true}))
}
