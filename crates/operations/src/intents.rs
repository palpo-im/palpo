//! Translate bounded form intents into authority-bound workflow requests.
use palpo_hagency_contract::{MatrixUserId, RequestId, TokenTopUpRequest, Tokens};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workflow::{Request, Workflows};
use crate::{Result, digest, fail};

pub(crate) fn top_up(
    workflows: &mut Workflows,
    state: &Value,
    input: Value,
    actor: &MatrixUserId,
    now: u64,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Intent {
        kind: String,
        agent_action_id: String,
        request_id: RequestId,
        expected_allocated_tokens: Tokens,
        requested_additional_tokens: String,
    }
    let fingerprint = digest(&json!({"actor":actor,"intent":input}))?;
    let input: Intent = serde_json::from_value(input)?;
    if input.kind != "token_top_up" {
        return Err(fail(400, "invalid_request_kind"));
    }
    let key = digest(&json!({"actor":actor,"requestId":input.request_id}))?;
    if let Some(receipt) = workflows.submission_intents.get(&key) {
        if receipt["digest"] != fingerprint {
            return Err(fail(409, "idempotency_conflict"));
        }
        return workflows.view(receipt["actionId"].as_str().unwrap_or_default(), actor, now);
    }
    let additional = input
        .requested_additional_tokens
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == input.requested_additional_tokens)
        .ok_or_else(|| fail(400, "invalid_token_count"))?;
    let additional = Tokens::try_from(additional)?;
    let action = workflows
        .actions
        .get(&input.agent_action_id)
        .ok_or_else(|| fail(404, "action_not_found"))?;
    if !crate::views::can_request_top_up(state, workflows, action, actor, now) {
        return Err(fail(409, "agent_allocation_unavailable"));
    }
    let Request::Agent(agent) = &action.request else {
        return Err(fail(400, "agent_required"));
    };
    let observation = &workflows.observations[&action.id];
    if observation["allocatedTokens"] != json!(input.expected_allocated_tokens) {
        return Err(fail(409, "agent_allocation_changed"));
    }
    let total = u64::from(input.expected_allocated_tokens)
        .checked_add(u64::from(additional))
        .ok_or_else(|| fail(400, "invalid_token_count"))?;
    if total
        > u64::from(
            workflows.authority.resources[agent.resource_allocation_id.as_str()].allocated_tokens,
        )
    {
        return Err(fail(409, "insufficient_capacity"));
    }
    let allocation = observation["engagementId"]
        .as_str()
        .ok_or_else(|| fail(409, "agent_allocation_unavailable"))?;
    let definition = json!({"agentAllocationId":allocation,"expectedAllocatedTokens":input.expected_allocated_tokens,
        "requestedAdditionalTokens":additional});
    let request = TokenTopUpRequest {
        id: input.request_id,
        revision: 1.try_into()?,
        server_engagement_id: agent.server_engagement_id.clone(),
        project_id: agent.project_id.clone(),
        project_revision: agent.project_revision,
        resource_allocation_id: agent.resource_allocation_id.clone(),
        agent_allocation_id: allocation.to_owned().try_into()?,
        project_owner: agent.project_owner.clone(),
        requester: actor.clone(),
        expected_allocated_tokens: input.expected_allocated_tokens,
        requested_additional_tokens: additional,
        definition_digest: digest(&definition)?.try_into()?,
    };
    let view = workflows.submit_definition(Request::TokenTopUp(request), definition, actor, now)?;
    workflows
        .submission_intents
        .insert(key, json!({"digest":fingerprint,"actionId":view["id"]}));
    Ok(view)
}
