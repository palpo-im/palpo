use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementState {
    Requested,
    Approved,
    Configuring,
    Verifying,
    Verified,
    Suspended,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    PendingCoordinator,
    Approved,
    Preparing,
    Ready,
    Rejected,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    PendingCoordinator,
    ApprovedWaitingHagency,
    AllocationRefused,
    Provisioning,
    ProvisioningFailed,
    Ready,
    Paused,
    Rejected,
    Retired,
}

/// Trusted current state loaded by the adapter, not request-body authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerEngagement {
    pub id: ServerEngagementId,
    pub server: ServerName,
    pub owner: MatrixUserId,
    pub coordinator: MatrixUserId,
    pub registration_generation: Revision,
    pub delegation_revision: Revision,
    pub delegation_expires_at_ms: u64,
    pub state: EngagementState,
    pub allow_self_approval: bool,
    pub coordinator_approval_v1: bool,
}

/// The server adapter supplies `is_current_server_admin` after authentication.
/// This check grants association authority only, not project/agent authority.
pub fn authorize_association(
    actor: &MatrixUserId,
    designated_admin: &MatrixUserId,
    server: &ServerName,
    is_current_server_admin: bool,
) -> Result<(), Error> {
    if !is_current_server_admin || actor != designated_admin || !actor.belongs_to(server) {
        return Err(Error::Forbidden);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandContext {
    pub version: u16,
    pub command_id: CommandId,
    pub server_engagement_id: ServerEngagementId,
    pub registration_generation: Revision,
    pub delegation_revision: Revision,
    pub actor: MatrixUserId,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

fn context(
    command: &CommandContext,
    engagement: &ServerEngagement,
    authenticated_actor: &MatrixUserId,
    now_ms: u64,
) -> Result<(), Error> {
    if command.version != CONTRACT_VERSION || !engagement.coordinator_approval_v1 {
        return Err(Error::Unsupported);
    }
    if command.actor != *authenticated_actor || !authenticated_actor.belongs_to(&engagement.server)
    {
        return Err(Error::Forbidden);
    }
    if command.server_engagement_id != engagement.id
        || command.registration_generation != engagement.registration_generation
        || command.delegation_revision != engagement.delegation_revision
        || !engagement.owner.belongs_to(&engagement.server)
        || !engagement.coordinator.belongs_to(&engagement.server)
    {
        return Err(Error::BindingMismatch);
    }
    if engagement.state != EngagementState::Verified {
        return Err(Error::EngagementUnavailable);
    }
    if [
        command.issued_at_ms,
        command.expires_at_ms,
        engagement.delegation_expires_at_ms,
    ]
    .into_iter()
    .any(|value| value > MAX_EXACT_JSON_INTEGER)
    {
        return Err(Error::InvalidNumber);
    }
    if command.issued_at_ms > now_ms
        || command.expires_at_ms <= now_ms
        || command.expires_at_ms > engagement.delegation_expires_at_ms
        || engagement.delegation_expires_at_ms <= now_ms
    {
        return Err(Error::Expired);
    }
    Ok(())
}

fn no_self_approval(
    actor: &MatrixUserId,
    requester: &MatrixUserId,
    engagement: &ServerEngagement,
) -> Result<(), Error> {
    if actor == requester && !engagement.allow_self_approval {
        return Err(Error::SelfApproval);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectRequest {
    pub id: RequestId,
    pub revision: Revision,
    pub server_engagement_id: ServerEngagementId,
    pub project_id: ProjectId,
    pub owner: MatrixUserId,
    pub requester: MatrixUserId,
    /// Immutable project definition including name, purpose and room binding.
    pub definition_digest: DefinitionDigest,
    pub resource_allocations: Vec<ResourceAllocationId>,
}

/// Frozen request equality protects every requested field, not just its ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectApproval {
    pub context: CommandContext,
    pub request: ProjectRequest,
}

pub fn authorize_project_approval(
    approval: &ProjectApproval,
    current_request: &ProjectRequest,
    engagement: &ServerEngagement,
    authenticated_actor: &MatrixUserId,
    now_ms: u64,
) -> Result<(), Error> {
    context(&approval.context, engagement, authenticated_actor, now_ms)?;
    if authenticated_actor != &engagement.coordinator {
        return Err(Error::Forbidden);
    }
    let request = &approval.request;
    if request != current_request
        || request.server_engagement_id != engagement.id
        || !request.owner.belongs_to(&engagement.server)
        || !request.requester.belongs_to(&engagement.server)
    {
        return Err(Error::BindingMismatch);
    }
    if request.resource_allocations.is_empty()
        || request.resource_allocations.len() > 64
        || request
            .resource_allocations
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != request.resource_allocations.len()
    {
        return Err(Error::ResourceNotGranted);
    }
    no_self_approval(authenticated_actor, &request.requester, engagement)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectGrant {
    pub project_id: ProjectId,
    pub server_engagement_id: ServerEngagementId,
    pub revision: Revision,
    pub owner: MatrixUserId,
    pub resource_allocations: Vec<ResourceAllocationId>,
    pub state: ProjectState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRequest {
    pub id: RequestId,
    pub revision: Revision,
    pub server_engagement_id: ServerEngagementId,
    pub project_id: ProjectId,
    pub project_revision: Revision,
    pub resource_allocation_id: ResourceAllocationId,
    pub project_owner: MatrixUserId,
    pub requester: MatrixUserId,
    /// SHA-256 of the immutable full agent definition, including room bindings,
    /// model/settings and requested rate/concurrency. Both adapters must use the
    /// same canonical encoding before this contract is enabled.
    pub definition_digest: DefinitionDigest,
    pub requested_tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentApproval {
    pub context: CommandContext,
    pub request: AgentRequest,
    pub allocated_tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenTopUpRequest {
    pub id: RequestId,
    pub revision: Revision,
    pub server_engagement_id: ServerEngagementId,
    pub project_id: ProjectId,
    pub project_revision: Revision,
    pub resource_allocation_id: ResourceAllocationId,
    pub agent_allocation_id: AgentAllocationId,
    pub project_owner: MatrixUserId,
    pub requester: MatrixUserId,
    pub definition_digest: DefinitionDigest,
    pub expected_allocated_tokens: Tokens,
    pub requested_additional_tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenTopUpApproval {
    pub context: CommandContext,
    pub request: TokenTopUpRequest,
    pub additional_tokens: Tokens,
}

/// The adapter also checks the current agent's immutable project/resource/owner
/// binding and allocation under its writer. This policy does not reserve tokens.
pub fn authorize_token_top_up(
    approval: &TokenTopUpApproval,
    current_request: &TokenTopUpRequest,
    engagement: &ServerEngagement,
    project: &ProjectGrant,
    authenticated_actor: &MatrixUserId,
    now_ms: u64,
) -> Result<(), Error> {
    if &approval.request != current_request {
        return Err(Error::BindingMismatch);
    }
    let request = &approval.request;
    let proposed = AgentApproval {
        context: approval.context.clone(),
        request: AgentRequest {
            id: request.id.clone(),
            revision: request.revision,
            server_engagement_id: request.server_engagement_id.clone(),
            project_id: request.project_id.clone(),
            project_revision: request.project_revision,
            resource_allocation_id: request.resource_allocation_id.clone(),
            project_owner: request.project_owner.clone(),
            requester: request.requester.clone(),
            definition_digest: request.definition_digest.clone(),
            requested_tokens: request.requested_additional_tokens,
        },
        allocated_tokens: approval.additional_tokens,
    };
    authorize_agent_approval(
        &proposed,
        &proposed.request,
        engagement,
        project,
        authenticated_actor,
        now_ms,
    )?;
    let total = u64::from(request.expected_allocated_tokens)
        .checked_add(u64::from(approval.additional_tokens))
        .ok_or(Error::Overflow)?;
    Tokens::try_from(total)?;
    Ok(())
}

/// A policy result constructed only by the current-state check. It is not a
/// durable decision or a capacity reservation and cannot be deserialized.
#[derive(Debug)]
pub struct AuthorizedAgentApproval<'a>(&'a AgentApproval);

impl<'a> AuthorizedAgentApproval<'a> {
    pub fn command(&self) -> &'a AgentApproval {
        self.0
    }
}

pub fn authorize_agent_approval<'a>(
    approval: &'a AgentApproval,
    current_request: &AgentRequest,
    engagement: &ServerEngagement,
    project: &ProjectGrant,
    authenticated_actor: &MatrixUserId,
    now_ms: u64,
) -> Result<AuthorizedAgentApproval<'a>, Error> {
    context(&approval.context, engagement, authenticated_actor, now_ms)?;
    if authenticated_actor != &engagement.coordinator && authenticated_actor != &engagement.owner {
        return Err(Error::Forbidden);
    }
    let request = &approval.request;
    if request != current_request
        || request.server_engagement_id != engagement.id
        || project.server_engagement_id != engagement.id
        || request.project_id != project.project_id
        || request.project_revision != project.revision
        || request.project_owner != project.owner
        || !request.project_owner.belongs_to(&engagement.server)
        || !request.requester.belongs_to(&engagement.server)
    {
        return Err(Error::BindingMismatch);
    }
    if project.state != ProjectState::Ready {
        return Err(Error::ProjectUnavailable);
    }
    if !project
        .resource_allocations
        .contains(&request.resource_allocation_id)
    {
        return Err(Error::ResourceNotGranted);
    }
    if u64::from(request.requested_tokens) == 0
        || u64::from(approval.allocated_tokens) == 0
        || approval.allocated_tokens > request.requested_tokens
    {
        return Err(Error::InvalidNumber);
    }
    no_self_approval(authenticated_actor, &request.requester, engagement)?;
    Ok(AuthorizedAgentApproval(approval))
}
