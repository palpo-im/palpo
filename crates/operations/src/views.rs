//! Role-scoped mini-app read models. Approval, readiness and metering are
//! separate evidence; missing or old observations never imply unused capacity.
use palpo_hagency_contract::{EngagementState, MatrixUserId, ProjectState};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::workflow::{Action, Request, Workflows};
use crate::{Result, fail};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    50
}
impl Page {
    fn apply(&self, key: &str, mut rows: Vec<Value>) -> Result<Value> {
        if !(1..=100).contains(&self.limit) {
            return Err(fail(400, "invalid_page"));
        }
        rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let total = rows.len();
        let mut result = json!({"total":total,"offset":self.offset,"limit":self.limit});
        result[key] = json!(
            rows.into_iter()
                .skip(self.offset)
                .take(self.limit)
                .collect::<Vec<_>>()
        );
        Ok(result)
    }
}

fn display(value: &Value, fallback: &str, limit: usize) -> String {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

pub(crate) fn can_request_top_up(
    state: &Value,
    workflows: &Workflows,
    action: &Action,
    actor: &MatrixUserId,
    now: u64,
) -> bool {
    let Request::Agent(agent) = &action.request else {
        return false;
    };
    let Some(observed) = workflows.observations.get(&action.id) else {
        return false;
    };
    let fleet = &state["fleets"][agent.server_engagement_id.as_str()];
    action.state == "approved"
        && agent.project_owner == *actor
        && observed["state"] == "active"
        && observed["engagementId"].as_str().is_some()
        && observed["allocatedTokens"].as_u64().is_some()
        && crate::updates::status_current(observed, now)
        && observed["generation"].as_u64().is_some()
        && observed["generation"] == fleet["transport"]["generation"]
        && fleet["state"] == "ready"
        && fleet["installation"] == "installed"
        && workflows
            .authority
            .engagements
            .get(agent.server_engagement_id.as_str())
            .is_some_and(|e| {
                e.state == EngagementState::Verified
                    && e.coordinator_approval_v1
                    && e.delegation_expires_at_ms > now
                    && u64::from(e.registration_generation)
                        == fleet["registrationGeneration"].as_u64().unwrap_or(1)
            })
        && workflows
            .authority
            .projects
            .get(agent.project_id.as_str())
            .is_some_and(|p| {
                p.state == ProjectState::Ready
                    && p.revision == agent.project_revision
                    && p.owner == *actor
                    && p.server_engagement_id == agent.server_engagement_id
                    && p.resource_allocations
                        .contains(&agent.resource_allocation_id)
            })
        && workflows
            .authority
            .resources
            .get(agent.resource_allocation_id.as_str())
            .is_some_and(|r| {
                r.server_engagement_id == agent.server_engagement_id
                    && u64::from(r.allocated_tokens) > 0
                    && r.eligible_managers.contains(actor)
            })
}

pub(crate) fn agents(
    state: &Value,
    workflows: &Workflows,
    actor: &MatrixUserId,
    now: u64,
    page: Page,
) -> Result<Value> {
    let mut rows = Vec::new();
    for action in workflows.actions.values() {
        let Request::Agent(request) = &action.request else {
            continue;
        };
        let view = match workflows.view(&action.id, actor, now) {
            Ok(view) => view,
            Err(e) if e.status == 404 => continue,
            Err(e) => return Err(e),
        };
        let observed = &view["result"];
        let fleet = &state["fleets"][request.server_engagement_id.as_str()];
        let current_binding = workflows
            .authority
            .engagements
            .get(request.server_engagement_id.as_str())
            .is_some_and(|e| {
                e.state == EngagementState::Verified
                    && fleet["registrationGeneration"].as_u64().unwrap_or(1)
                        == u64::from(e.registration_generation)
            })
            && observed["generation"].as_u64().is_some()
            && observed["generation"] == fleet["transport"]["generation"];
        let current = current_binding && crate::updates::status_current(observed, now);
        let usable = current
            && view["execution"] == "ready"
            && observed["ready"] == true
            && observed["bound"] == true
            && observed["quotaPaused"] != true
            && fleet["installation"] == "installed"
            && fleet["state"] == "ready"
            && workflows
                .authority
                .projects
                .get(request.project_id.as_str())
                .is_some_and(|p| {
                    p.state == ProjectState::Ready
                        && p.revision == request.project_revision
                        && p.owner == request.project_owner
                        && p.server_engagement_id == request.server_engagement_id
                });
        let execution = if view["execution"] == "ready" && !usable {
            if current && observed["quotaPaused"] == true {
                "paused"
            } else {
                "unknown"
            }
        } else {
            view["execution"].as_str().unwrap_or("unknown")
        };
        let usage_time = observed["usageObservedAtMs"].as_u64().filter(|at| {
            *at <= now.saturating_add(5000)
                && observed["receivedAtMs"]
                    .as_u64()
                    .is_some_and(|received| *at <= received.saturating_add(5000))
        });
        // The native meter currently reports an attributed lower bound. Do not
        // turn it into an exact balance, even when the sample is recent.
        let consumed = observed["consumedTokens"].as_u64().filter(|n| {
            *n <= 9_007_199_254_740_991
                && usage_time.is_some()
                && observed["usageEvidence"] == "host_attributed_lower_bound"
        });
        let usage_state = if consumed.is_none() {
            "unknown"
        } else if !current || usage_time.is_none_or(|at| now.saturating_sub(at) >= 90000) {
            "stale"
        } else {
            "current"
        };
        rows.push(json!({"id":action.id,"requestId":request.id,"projectId":request.project_id,
            "fleetId":request.server_engagement_id,"ownerMxid":request.project_owner,"state":action.state,
            "execution":execution,"failureReason":view["failureReason"],"usable":usable,"statusFresh":current,
            "agentDefinition":{"name":display(&view["payload"]["name"],request.id.as_str(),160)},
            "role":display(&view["payload"]["role"],"agent",80),"requestedTokens":request.requested_tokens,
            "allocatedTokens":observed["allocatedTokens"].as_u64(),
            "agentMxid":observed["agentMxid"].as_str(),
            "usage":{"state":usage_state,"consumedTokens":consumed,"observedAtMs":usage_time,
                "evidence":if consumed.is_some(){"host_attributed_lower_bound"}else{"unknown"},"complete":false},
            "quotaPaused":observed["quotaPaused"]==true,"jobSummary":null}));
        rows.last_mut().unwrap()["canRequestTopUp"] =
            json!(can_request_top_up(state, workflows, action, actor, now));
    }
    page.apply("requests", rows)
}

pub(crate) fn projects(
    state: &Value,
    workflows: &Workflows,
    actor: &MatrixUserId,
    now: u64,
    page: Page,
) -> Result<Value> {
    let mut rows = Vec::new();
    for project in workflows.authority.projects.values() {
        let reviewer = workflows
            .authority
            .engagements
            .get(project.server_engagement_id.as_str())
            .is_some_and(|e| {
                e.state == EngagementState::Verified
                    && e.delegation_expires_at_ms > now
                    && (e.coordinator == *actor || e.owner == *actor)
            });
        if project.owner != *actor && !reviewer {
            continue;
        }
        let definition = workflows.actions.values().find_map(|a| match &a.request {
            Request::Project(r)
                if r.project_id == project.project_id
                    && r.server_engagement_id == project.server_engagement_id
                    && r.revision == project.revision =>
            {
                workflows
                    .definitions
                    .get(&String::from(r.definition_digest.clone()))
            }
            _ => None,
        });
        rows.push(json!({"id":project.project_id,"fleetId":project.server_engagement_id,"state":project.state,
            "ownerMxid":project.owner,"name":display(&definition.map_or(Value::Null, |d|d["name"].clone()),project.project_id.as_str(),160),
            "resourceAllocationIds":project.resource_allocations,
            "canRequest":project.owner==*actor && project.state==ProjectState::Ready && definition.is_some()
                && engagement_available(state,workflows,project.server_engagement_id.as_str(),now)}));
    }
    page.apply("projects", rows)
}

pub(crate) fn engagement_available(
    state: &Value,
    workflows: &Workflows,
    id: &str,
    now: u64,
) -> bool {
    let fleet = &state["fleets"][id];
    fleet["installation"] == "installed"
        && fleet["state"] == "ready"
        && fleet["capabilities"]["coordinatorApprovalV1"] == true
        && workflows.authority.engagements.get(id).is_some_and(|e| {
            e.state == EngagementState::Verified
                && e.coordinator_approval_v1
                && e.delegation_expires_at_ms > now
                && u64::from(e.registration_generation)
                    == fleet["registrationGeneration"].as_u64().unwrap_or(1)
        })
}

/// Only funded grants published by this engagement are requestable. Expose the
/// allocation separately from its underlying resource so two engagements (or
/// two periods) cannot be accidentally treated as one budget.
pub(crate) fn catalog(
    state: &Value,
    workflows: &Workflows,
    actor: &MatrixUserId,
    now: u64,
    input: Value,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Filter {
        project_id: Option<String>,
    }
    let filter: Filter = serde_json::from_value(input)?;
    let project = filter
        .project_id
        .as_ref()
        .map(|id| {
            workflows
                .authority
                .projects
                .get(id)
                .filter(|p| p.owner == *actor && p.state == ProjectState::Ready)
                .ok_or_else(|| fail(404, "project_not_found"))
        })
        .transpose()?;
    let mut fleets = Vec::new();
    for engagement in workflows.authority.engagements.values() {
        let id = engagement.id.as_str();
        if !engagement_available(state, workflows, id, now)
            || project.is_some_and(|p| p.server_engagement_id != engagement.id)
        {
            continue;
        }
        let fleet = &state["fleets"][id];
        let mut offers = Vec::new();
        for offer in fleet["capabilities"]["offers"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut resources = Vec::new();
            for resource in offer["resources"].as_array().into_iter().flatten() {
                for grant in workflows.authority.resources.values() {
                    if grant.server_engagement_id != engagement.id
                        || u64::from(grant.allocated_tokens) == 0
                        || !grant.eligible_managers.contains(actor)
                        || project.is_some_and(|p| !p.resource_allocations.contains(&grant.id))
                    {
                        continue;
                    }
                    let Some(details) = workflows.resource_details.get(grant.id.as_str()) else {
                        continue;
                    };
                    if details["resourceId"].as_str().is_none()
                        || details["resourceId"] != resource["id"]
                    {
                        continue;
                    }
                    let mut row = resource.clone();
                    row["allocationId"] = json!(grant.id);
                    row["allocationRevision"] = json!(grant.revision);
                    row["allocatedTokens"] = json!(grant.allocated_tokens);
                    row["period"] = details["period"].clone();
                    row["periodKey"] = details["periodKey"].clone();
                    resources.push(row);
                }
            }
            if !resources.is_empty() {
                offers.push(json!({"role":offer["role"],"description":offer["description"],"resources":resources}));
            }
        }
        if !offers.is_empty() {
            fleets.push(
                json!({"id":id,"name":display(&fleet["name"],id,160),"state":fleet["state"],
            "capabilities":{"offers":offers}}),
            );
        }
    }
    Ok(json!({"fleets":fleets}))
}
