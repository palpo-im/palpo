use palpo_hagency_contract::budget::BudgetSnapshot;
use palpo_hagency_contract::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn parse<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

fn user(name: &str) -> MatrixUserId {
    format!("@{name}:example.test").try_into().unwrap()
}

fn engagement() -> ServerEngagement {
    parse(json!({
        "id": "server_engagement_a", "server": "example.test",
        "owner": "@provider:example.test", "coordinator": "@coordinator:example.test",
        "registrationGeneration": 2, "delegationRevision": 3,
        "delegationExpiresAtMs": 5000, "state": "verified",
        "allowSelfApproval": false, "coordinatorApprovalV1": true
    }))
}

fn project() -> ProjectGrant {
    parse(json!({
        "projectId": "project_a", "serverEngagementId": "server_engagement_a",
        "revision": 4, "owner": "@manager:example.test",
        "resourceAllocations": ["grant_a"], "state": "ready"
    }))
}

fn approval() -> AgentApproval {
    parse(json!({
        "context": {
            "version": 1, "commandId": "decision_a", "serverEngagementId": "server_engagement_a",
            "registrationGeneration": 2, "delegationRevision": 3,
            "actor": "@coordinator:example.test", "issuedAtMs": 1000, "expiresAtMs": 3000
        },
        "request": {
            "id": "request_a", "revision": 5, "serverEngagementId": "server_engagement_a",
            "projectId": "project_a", "projectRevision": 4, "resourceAllocationId": "grant_a",
            "projectOwner": "@manager:example.test", "requester": "@manager:example.test",
            "definitionDigest": "a".repeat(64), "requestedTokens": 100000
        },
        "allocatedTokens": 100000
    }))
}

fn authorize(
    approval: &AgentApproval,
    current: &AgentRequest,
    engagement: &ServerEngagement,
    project: &ProjectGrant,
    actor: &MatrixUserId,
) -> Result<(), Error> {
    authorize_agent_approval(approval, current, engagement, project, actor, 2000).map(|_| ())
}

fn project_approval() -> ProjectApproval {
    ProjectApproval {
        context: approval().context,
        request: parse(json!({
            "id": "project_request", "revision": 1, "serverEngagementId": "server_engagement_a",
            "projectId": "project_a", "owner": "@manager:example.test",
            "requester": "@manager:example.test", "resourceAllocations": ["grant_a"],
            "definitionDigest": "b".repeat(64)
        })),
    }
}

#[test]
fn coordinator_decision_suffices_without_console_approval_data() {
    let command = approval();
    let result = authorize_agent_approval(
        &command,
        &command.request,
        &engagement(),
        &project(),
        &user("coordinator"),
        2000,
    )
    .unwrap();
    assert_eq!(result.command(), &command);
    let command = project_approval();
    assert_eq!(
        authorize_project_approval(
            &command,
            &command.request,
            &engagement(),
            &user("coordinator"),
            2000
        ),
        Ok(())
    );
}

#[test]
fn top_up_requires_current_delegation_frozen_intent_and_bounded_addition() {
    let mut request = serde_json::to_value(approval().request).unwrap();
    request.as_object_mut().unwrap().remove("requestedTokens");
    request["agentAllocationId"] = json!("en_littlewhite");
    request["expectedAllocatedTokens"] = json!(100000);
    request["requestedAdditionalTokens"] = json!(50000);
    let command = TokenTopUpApproval {
        context: approval().context,
        request: parse(request),
        additional_tokens: 50000.try_into().unwrap(),
    };
    assert_eq!(
        authorize_token_top_up(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator"),
            2000
        ),
        Ok(())
    );
    let mut changed = command.clone();
    changed.request.agent_allocation_id = "different".to_owned().try_into().unwrap();
    assert_eq!(
        authorize_token_top_up(
            &changed,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator"),
            2000
        ),
        Err(Error::BindingMismatch)
    );
    changed = command.clone();
    changed.additional_tokens = 50001.try_into().unwrap();
    assert_eq!(
        authorize_token_top_up(
            &changed,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator"),
            2000
        ),
        Err(Error::InvalidNumber)
    );
    changed = command.clone();
    changed.context.actor = user("admin");
    assert_eq!(
        authorize_token_top_up(
            &changed,
            &command.request,
            &engagement(),
            &project(),
            &user("admin"),
            2000
        ),
        Err(Error::Forbidden)
    );
    let mut authority = engagement();
    authority.delegation_revision = 4.try_into().unwrap();
    assert_eq!(
        authorize_token_top_up(
            &command,
            &command.request,
            &authority,
            &project(),
            &user("coordinator"),
            2000
        ),
        Err(Error::BindingMismatch)
    );
    assert_eq!(
        authorize_token_top_up(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator"),
            3000
        ),
        Err(Error::Expired)
    );
    changed = command;
    changed.request.expected_allocated_tokens = MAX_EXACT_JSON_INTEGER.try_into().unwrap();
    assert!(
        authorize_token_top_up(
            &changed,
            &changed.request,
            &engagement(),
            &project(),
            &user("coordinator"),
            2000
        )
        .is_err()
    );
}

#[test]
fn server_admin_association_authority_does_not_confer_resource_authority() {
    let admin = user("admin");
    let server: ServerName = "example.test".to_owned().try_into().unwrap();
    assert_eq!(authorize_association(&admin, &admin, &server, true), Ok(()));
    assert_eq!(
        authorize_association(&admin, &admin, &server, false),
        Err(Error::Forbidden)
    );
    assert_eq!(
        authorize_association(&user("other_admin"), &admin, &server, true),
        Err(Error::Forbidden)
    );

    for name in ["admin", "manager", "other_coordinator"] {
        let mut command = approval();
        command.context.actor = user(name);
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &engagement(),
                &project(),
                &user(name)
            ),
            Err(Error::Forbidden)
        );
        let mut command = project_approval();
        command.context.actor = user(name);
        assert_eq!(
            authorize_project_approval(
                &command,
                &command.request,
                &engagement(),
                &user(name),
                2000
            ),
            Err(Error::Forbidden)
        );
    }
}

#[test]
fn owner_may_decide_agents_but_projects_require_coordinator_assignment() {
    let mut command = approval();
    command.context.actor = user("provider");
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("provider")
        ),
        Ok(())
    );
    let mut command = project_approval();
    command.context.actor = user("provider");
    assert_eq!(
        authorize_project_approval(
            &command,
            &command.request,
            &engagement(),
            &user("provider"),
            2000
        ),
        Err(Error::Forbidden)
    );
    let mut current = engagement();
    current.coordinator = user("provider");
    assert_eq!(
        authorize_project_approval(
            &command,
            &command.request,
            &current,
            &user("provider"),
            2000
        ),
        Ok(())
    );
}

#[test]
fn same_homeserver_does_not_make_engagements_interchangeable() {
    let command = approval();
    let mut other = engagement();
    other.id = "server_engagement_b".to_owned().try_into().unwrap();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &other,
            &project(),
            &user("coordinator")
        ),
        Err(Error::BindingMismatch)
    );
    let mut other_project = project();
    other_project.server_engagement_id = other.id;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &other_project,
            &user("coordinator")
        ),
        Err(Error::BindingMismatch)
    );
}

#[test]
fn request_content_is_frozen_including_digest_amount_and_owner() {
    let command = approval();
    for (key, replacement) in [
        ("id", json!("other_request")),
        ("revision", json!(6)),
        ("serverEngagementId", json!("server_engagement_b")),
        ("projectId", json!("project_b")),
        ("projectRevision", json!(5)),
        ("resourceAllocationId", json!("grant_b")),
        ("projectOwner", json!("@other:example.test")),
        ("requester", json!("@other:example.test")),
        ("definitionDigest", json!("b".repeat(64))),
        ("requestedTokens", json!(200000)),
    ] {
        let mut current = serde_json::to_value(&command.request).unwrap();
        current[key] = replacement;
        let current = parse(current);
        assert_eq!(
            authorize(
                &command,
                &current,
                &engagement(),
                &project(),
                &user("coordinator")
            ),
            Err(Error::BindingMismatch),
            "{key}"
        );
    }
}

#[test]
fn persisted_decision_must_be_rechecked_after_delegation_or_registration_changes() {
    let command = approval();
    for field in ["registrationGeneration", "delegationRevision"] {
        let mut current = serde_json::to_value(engagement()).unwrap();
        current[field] = json!(100);
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &parse(current),
                &project(),
                &user("coordinator")
            ),
            Err(Error::BindingMismatch)
        );
    }
    for state in [
        EngagementState::Requested,
        EngagementState::Approved,
        EngagementState::Configuring,
        EngagementState::Verifying,
        EngagementState::Suspended,
        EngagementState::Revoked,
    ] {
        let mut current = engagement();
        current.state = state;
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &current,
                &project(),
                &user("coordinator")
            ),
            Err(Error::EngagementUnavailable)
        );
    }
    let mut current = engagement();
    current.coordinator = user("replacement");
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("coordinator")
        ),
        Err(Error::Forbidden)
    );
}

#[test]
fn unsupported_runtime_cannot_fall_back_to_console_or_auto_approval() {
    let mut command = approval();
    let mut current = engagement();
    current.coordinator_approval_v1 = false;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("coordinator")
        ),
        Err(Error::Unsupported)
    );
    command.context.version = 2;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator")
        ),
        Err(Error::Unsupported)
    );
}

#[test]
fn command_and_delegation_expiry_are_execution_gates() {
    let command = approval();
    for (issued, expires) in [(2001, 3000), (1000, 2000), (1000, 6000)] {
        let mut command = command.clone();
        command.context.issued_at_ms = issued;
        command.context.expires_at_ms = expires;
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &engagement(),
                &project(),
                &user("coordinator")
            ),
            Err(Error::Expired)
        );
    }
    let mut current = engagement();
    current.delegation_expires_at_ms = 2000;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("coordinator")
        ),
        Err(Error::Expired)
    );
}

#[test]
fn body_actor_is_not_authenticated_identity() {
    let command = approval();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("manager")
        ),
        Err(Error::Forbidden)
    );
    let mut command = command;
    command.context.actor = "@coordinator:elsewhere.test".to_owned().try_into().unwrap();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &command.context.actor
        ),
        Err(Error::Forbidden)
    );
}

#[test]
fn approved_project_is_not_yet_ready_and_withdrawn_grants_are_refused() {
    let command = approval();
    for state in [
        ProjectState::PendingCoordinator,
        ProjectState::Approved,
        ProjectState::Preparing,
        ProjectState::Rejected,
        ProjectState::Revoked,
    ] {
        let mut current = project();
        current.state = state;
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &engagement(),
                &current,
                &user("coordinator")
            ),
            Err(Error::ProjectUnavailable)
        );
    }
    let mut current = project();
    current.resource_allocations.clear();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &current,
            &user("coordinator")
        ),
        Err(Error::ResourceNotGranted)
    );
    let mut current = project();
    current.revision = 5.try_into().unwrap();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &current,
            &user("coordinator")
        ),
        Err(Error::BindingMismatch)
    );
}

#[test]
fn amount_changes_do_not_silently_increase_the_request() {
    let mut command = approval();
    command.allocated_tokens = 50000.try_into().unwrap();
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &engagement(),
            &project(),
            &user("coordinator")
        ),
        Ok(())
    );
    for amount in [0, 100001] {
        command.allocated_tokens = amount.try_into().unwrap();
        assert_eq!(
            authorize(
                &command,
                &command.request,
                &engagement(),
                &project(),
                &user("coordinator")
            ),
            Err(Error::InvalidNumber)
        );
    }
}

#[test]
fn self_approval_requires_explicit_owner_policy() {
    let mut command = approval();
    let mut current = engagement();
    current.coordinator = user("manager");
    command.context.actor = user("manager");
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("manager")
        ),
        Err(Error::SelfApproval)
    );
    current.allow_self_approval = true;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("manager")
        ),
        Ok(())
    );
}

#[test]
fn wire_rejects_unknown_fields_and_invalid_identifiers() {
    let mut value = serde_json::to_value(approval()).unwrap();
    value["consoleApproved"] = json!(true);
    assert!(serde_json::from_value::<AgentApproval>(value).is_err());
    for id in ["", "../other", "example.test", "a b"] {
        assert!(ServerEngagementId::try_from(id.to_owned()).is_err());
    }
    for id in [
        "@:example.test",
        "@ :example.test",
        "@user\n:example.test",
        "user",
        "@user:https://example.test",
    ] {
        assert!(MatrixUserId::try_from(id.to_owned()).is_err(), "{id}");
    }
    assert!(MatrixUserId::try_from("@user:[::1]:8448".to_owned()).is_ok());
    assert!(serde_json::from_value::<Revision>(json!(0)).is_err());
    assert!(serde_json::from_value::<Tokens>(json!(-1)).is_err());
    assert!(serde_json::from_value::<Tokens>(json!(MAX_EXACT_JSON_INTEGER + 1)).is_err());
    let max: Tokens = parse(json!(MAX_EXACT_JSON_INTEGER));
    assert_eq!(
        serde_json::to_value(max).unwrap(),
        json!(MAX_EXACT_JSON_INTEGER)
    );
    let command = approval();
    assert_eq!(
        parse::<AgentApproval>(serde_json::to_value(&command).unwrap()),
        command
    );
}

#[test]
fn consumption_does_not_free_reserved_capacity_and_retirement_cannot_refund_spend() {
    let before: BudgetSnapshot =
        parse(json!({"allocated": 1000000, "consumed": 0, "reservedUnused": 700000}));
    let consumed: BudgetSnapshot =
        parse(json!({"allocated": 1000000, "consumed": 100000, "reservedUnused": 600000}));
    let retired: BudgetSnapshot =
        parse(json!({"allocated": 1000000, "consumed": 100000, "reservedUnused": 400000}));
    assert_eq!(u64::from(before.available().unwrap()), 300000);
    assert_eq!(u64::from(consumed.available().unwrap()), 300000);
    assert_eq!(u64::from(retired.available().unwrap()), 500000);
    assert_eq!(retired.check_increase(500000.try_into().unwrap()), Ok(()));
    assert_eq!(
        retired.check_increase(500001.try_into().unwrap()),
        Err(Error::InsufficientCapacity)
    );
}

#[test]
fn missing_zero_and_overdrawn_budgets_never_authorize_an_increase() {
    for allocated in [Value::Null, json!(0), json!(50)] {
        let budget: BudgetSnapshot =
            parse(json!({"allocated": allocated, "consumed": 50, "reservedUnused": 50}));
        assert!(budget.check_increase(1.try_into().unwrap()).is_err());
    }
    let budget: BudgetSnapshot =
        parse(json!({"allocated": 100, "consumed": 10, "reservedUnused": 10}));
    assert_eq!(
        budget.check_increase(Tokens::default()),
        Err(Error::InvalidNumber)
    );
}

#[test]
fn project_decisions_bind_the_full_definition_and_refuse_duplicate_resources() {
    let command = project_approval();
    let mut current = command.request.clone();
    current.definition_digest = "c".repeat(64).try_into().unwrap();
    assert_eq!(
        authorize_project_approval(
            &command,
            &current,
            &engagement(),
            &user("coordinator"),
            2000
        ),
        Err(Error::BindingMismatch)
    );
    let mut command = command;
    command
        .request
        .resource_allocations
        .push(command.request.resource_allocations[0].clone());
    assert_eq!(
        authorize_project_approval(
            &command,
            &command.request,
            &engagement(),
            &user("coordinator"),
            2000
        ),
        Err(Error::ResourceNotGranted)
    );
    command.request.resource_allocations.clear();
    assert_eq!(
        authorize_project_approval(
            &command,
            &command.request,
            &engagement(),
            &user("coordinator"),
            2000
        ),
        Err(Error::ResourceNotGranted)
    );
}

#[test]
fn malformed_digests_and_inexact_timestamps_cannot_cross_the_contract() {
    for digest in [
        "".to_owned(),
        "a".repeat(63),
        "g".repeat(64),
        "A".repeat(64),
    ] {
        assert!(serde_json::from_value::<DefinitionDigest>(json!(digest)).is_err());
    }
    let mut command = approval();
    let mut current = engagement();
    command.context.expires_at_ms = MAX_EXACT_JSON_INTEGER + 1;
    current.delegation_expires_at_ms = MAX_EXACT_JSON_INTEGER + 2;
    assert_eq!(
        authorize(
            &command,
            &command.request,
            &current,
            &project(),
            &user("coordinator")
        ),
        Err(Error::InvalidNumber)
    );
}
