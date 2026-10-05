//! Read legacy Inbox records without reviving their superseded authority.
//! Continuation preserves the original source and uses current funded grants.
use crate::{Result, digest, fail, workflow::Workflows};
use palpo_hagency_contract::MatrixUserId;
use serde_json::{Value, json};

pub(crate) fn continuation(w: &Workflows, row: &Value, actor: &MatrixUserId, now: u64) -> Option<Value> {
    if row["kind"] != "project" || row["ownerMxid"] != actor.as_str()
        || !matches!(row["state"].as_str(), Some("requested" | "approved"))
        || row["execution"] == "done" || row["result"]["projectId"].is_string() {
        return None;
    }
    let fleet = row["payload"]["fleetId"].as_str()?;
    let authority = w.authority.engagements.get(fleet)?;
    if authority.state != palpo_hagency_contract::EngagementState::Verified
        || !authority.coordinator_approval_v1 || authority.delegation_expires_at_ms <= now {
        return None;
    }
    let mut allocations = Vec::new();
    for parent in row["payload"]["resourceIds"].as_array()? {
        let choices: Vec<_> = w.authority.resources.values().filter(|r|
            r.server_engagement_id.as_str() == fleet && r.eligible_managers.contains(actor)
            && u64::from(r.allocated_tokens) > 0 && w.resource_details[r.id.as_str()]["resourceId"] == *parent
        ).collect();
        if choices.len() != 1 { return None; }
        allocations.push(choices[0].id.clone());
    }
    if allocations.is_empty() || allocations.len() > 64 { return None; }
    allocations.sort(); allocations.dedup();
    let request_id = row["requestId"].as_str()?;
    let expected_id = format!("action_{}", &digest(&json!({"actor":actor,"requestId":request_id})).ok()?[..32]);
    if row["id"] != expected_id { return None; }
    let mut input = json!({"kind":"project","requestId":request_id,"fleetId":fleet,"resourceIds":allocations,
        "name":row["payload"]["name"],"reason":row["payload"]["reason"].as_str().unwrap_or_default()});
    if let Some(room) = row["payload"]["roomId"].as_str() { input["roomId"] = json!(room); }
    Some(input)
}

pub(crate) fn view(w: &Workflows, id: &str, actor: &MatrixUserId, now: u64) -> Result<Value> {
    let row = w.legacy_records.get(id).ok_or_else(||fail(404,"action_not_found"))?;
    let fleet = row["payload"]["fleetId"].as_str().or_else(||row["result"]["fleetId"].as_str());
    let authorized = row["ownerMxid"] == actor.as_str() || fleet.and_then(|id| w.authority.engagements.get(id)).is_some_and(|e|
        e.owner == *actor || e.coordinator == *actor && e.delegation_expires_at_ms > now);
    if !authorized { return Err(fail(404,"action_not_found")); }
    // Whitelist presentation fields: original records may contain command or
    // future extension data that is not an application result.
    let mut result = json!({"id":id,"kind":row["kind"],"ownerMxid":row["ownerMxid"],"fleetId":fleet,
        "state":row["state"],"execution":row["execution"],"revision":row["revision"],
        "createdAt":row["createdAt"],"updatedAt":row["updatedAt"],"legacy":true,
        "payload":{"name":row["payload"]["name"].as_str().unwrap_or("Previous request"),"reason":row["payload"]["reason"].as_str().unwrap_or_default()},
        "canDecide":false,"canContinue":false,"needsMyAction":false,"nextAction":null});
    if row["decision"].is_object() {
        result["decision"] = json!({"by":row["decision"]["by"],"at":row["decision"]["at"],"reason":row["decision"]["reason"]});
    }
    if let Some(input) = continuation(w,row,actor,now) {
        result["continuation"] = input;
        result["canContinue"] = json!(true);
        result["needsMyAction"] = json!(true);
        result["nextAction"] = json!("continue_legacy_project");
    }
    result["migrationNote"] = json!(if row["kind"] == "contribution" {
        "This earlier contribution and its decision are retained. Continue association setup from Hagency using the existing fleet ID; resource allocation now belongs to the Hagency owner."
    } else if result["canContinue"] == true {
        "Continue this request with its original identity and current funded resources. The assigned coordinator reviews it under the current policy; its earlier decision stays in history."
    } else {
        "This earlier request and its decision are retained. Unfinished work requires its Hagency owner to map the existing project and resources before it can continue."
    });
    Ok(result)
}
