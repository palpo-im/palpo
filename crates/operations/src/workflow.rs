use std::collections::{BTreeMap, BTreeSet};

use palpo_hagency_contract::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Result, digest, fail};

/// Trusted Hagency projection. This service never manufactures capacity from a
/// manager's project request. Import is an explicit local operator operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub id: ResourceAllocationId,
    pub server_engagement_id: ServerEngagementId,
    pub revision: Revision,
    pub allocated_tokens: Tokens,
    pub eligible_managers: Vec<MatrixUserId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthoritySnapshot {
    pub engagements: BTreeMap<String, ServerEngagement>,
    pub resources: BTreeMap<String, Resource>,
    pub projects: BTreeMap<String, ProjectGrant>,
}

impl AuthoritySnapshot {
    pub fn validate(&self, server: &ServerName) -> Result<()> {
        for (id, e) in &self.engagements {
            if id != e.id.as_str()
                || &e.server != server
                || !e.owner.belongs_to(server)
                || !e.coordinator.belongs_to(server)
            {
                return Err(fail(400, "invalid_authority_snapshot"));
            }
        }
        for (id, r) in &self.resources {
            if id != r.id.as_str()
                || !self
                    .engagements
                    .contains_key(r.server_engagement_id.as_str())
                || r.eligible_managers.iter().any(|u| !u.belongs_to(server))
            {
                return Err(fail(400, "invalid_authority_snapshot"));
            }
        }
        for (id, p) in &self.projects {
            if id != p.project_id.as_str()
                || !p.owner.belongs_to(server)
                || !self
                    .engagements
                    .contains_key(p.server_engagement_id.as_str())
                || p.resource_allocations.iter().any(|r| {
                    self.resources
                        .get(r.as_str())
                        .is_none_or(|r| r.server_engagement_id != p.server_engagement_id)
                })
            {
                return Err(fail(400, "invalid_authority_snapshot"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Request {
    Project(ProjectRequest),
    Agent(AgentRequest),
    TokenTopUp(TokenTopUpRequest),
}

impl Request {
    pub(crate) fn definition_digest(&self) -> &DefinitionDigest {
        match self {
            Self::Project(r) => &r.definition_digest,
            Self::Agent(r) => &r.definition_digest,
            Self::TokenTopUp(r) => &r.definition_digest,
        }
    }
    pub(crate) fn id(&self) -> &RequestId {
        match self {
            Self::Project(r) => &r.id,
            Self::Agent(r) => &r.id,
            Self::TokenTopUp(r) => &r.id,
        }
    }
    pub(crate) fn engagement(&self) -> &ServerEngagementId {
        match self {
            Self::Project(r) => &r.server_engagement_id,
            Self::Agent(r) => &r.server_engagement_id,
            Self::TokenTopUp(r) => &r.server_engagement_id,
        }
    }
    fn owner(&self) -> &MatrixUserId {
        match self {
            Self::Project(r) => &r.owner,
            Self::Agent(r) => &r.project_owner,
            Self::TokenTopUp(r) => &r.project_owner,
        }
    }
    fn requester(&self) -> &MatrixUserId {
        match self {
            Self::Project(r) => &r.requester,
            Self::Agent(r) => &r.requester,
            Self::TokenTopUp(r) => &r.requester,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub request: Request,
    pub state: String,
    pub revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub decision: Option<Value>,
    pub execution: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workflows {
    #[serde(default)]
    pub engagement_exports: BTreeMap<String, Vec<MatrixUserId>>,
    #[serde(default)]
    pub associations: BTreeMap<String, crate::associations::Association>,
    /// Runtime-published parent resource and accounting period for each grant.
    /// The allocation ID is the authority; a catalog resource alone grants none.
    #[serde(default)]
    pub resource_details: BTreeMap<String, Value>,
    #[serde(default)]
    pub submission_intents: BTreeMap<String, Value>,
    #[serde(default)]
    pub observations: BTreeMap<String, Value>,
    #[serde(default)]
    pub definitions: BTreeMap<String, Value>,
    pub authority: AuthoritySnapshot,
    pub actions: BTreeMap<String, Action>,
    pub receipts: BTreeMap<String, Value>,
    pub outbox: BTreeMap<String, Value>,
    pub notices: BTreeMap<String, Value>,
}

impl Workflows {
    pub(crate) fn may_export_profile(&self, fleet: &str, actor: &MatrixUserId) -> bool {
        self.engagement_exports.get(fleet).map_or_else(
            || {
                self.associations
                    .values()
                    .find(|a| a.fleet_id == fleet)
                    .is_some_and(|a| a.intent.export_mxids.contains(actor))
            },
            |recipients| recipients.contains(actor),
        )
    }
    pub fn submit_definition(
        &mut self,
        request: Request,
        definition: Value,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let expected: String = request.definition_digest().clone().into();
        if !definition.is_object()
            || serde_json::to_vec(&definition)?.len() > 16384
            || digest(&definition)? != expected
        {
            return Err(fail(400, "definition_digest_mismatch"));
        }
        let result = self.submit(request, actor, now)?;
        self.definitions.insert(expected, definition);
        self.view(
            result["id"]
                .as_str()
                .ok_or_else(|| fail(503, "workflow_state_invalid"))?,
            actor,
            now,
        )
    }

    pub(crate) fn enqueue_commands(
        &mut self,
        state: &Value,
        tx: &rusqlite::Transaction<'_>,
    ) -> Result<()> {
        for record in self
            .outbox
            .values_mut()
            .filter(|r| r["state"] == "pending" && r["queued"].is_null())
        {
            let id = record["actionId"]
                .as_str()
                .ok_or_else(|| fail(503, "workflow_state_invalid"))?;
            let action = self
                .actions
                .get(id)
                .ok_or_else(|| fail(503, "workflow_state_invalid"))?;
            let key: String = action.request.definition_digest().clone().into();
            let Some(definition) = self.definitions.get(&key) else {
                continue;
            };
            let fleet = &state["fleets"][action.request.engagement().as_str()];
            if fleet["installation"] != "installed"
                || fleet["state"] != "ready"
                || fleet["transport"]["mode"] != "outbound"
                || fleet["capabilities"]["coordinatorApprovalV1"] != true
            {
                return Err(fail(501, "coordinator_protocol_unavailable"));
            }
            let mut payload = match &action.request {
                Request::Project(_) => {
                    json!({"operation":"coordinator_project_approval","command":record["command"],"definition":definition})
                }
                Request::TokenTopUp(_) => {
                    json!({"operation":"coordinator_token_top_up","command":record["command"],"definition":definition})
                }
                Request::Agent(_) => {
                    let mut payload = definition.clone();
                    if payload["fleetId"] != fleet["id"] {
                        return Err(fail(409, "definition_binding_mismatch"));
                    }
                    payload["coordinatorApproval"] = record["command"].clone();
                    payload
                }
            };
            payload["coordinatorCommandId"] = record["id"].clone();
            crate::outbound::enqueue(
                tx,
                fleet,
                "work",
                "request",
                record["id"]
                    .as_str()
                    .ok_or_else(|| fail(503, "workflow_state_invalid"))?,
                &payload,
                crate::outbound::Limits::default(),
            )?;
            record["queued"] = json!(true);
            record["transportGeneration"] = fleet["transport"]["generation"].clone();
        }
        Ok(())
    }
    /// Offline operator migration, not an app-callable permission grant. The
    /// future authenticated Hagency projection worker must apply these same
    /// monotonic checks after verifying the source and registration binding.
    pub fn import_authority(
        &mut self,
        snapshot: AuthoritySnapshot,
        server: &ServerName,
    ) -> Result<()> {
        snapshot.validate(server)?;
        if self
            .authority
            .engagements
            .keys()
            .any(|id| !snapshot.engagements.contains_key(id))
            || self
                .authority
                .resources
                .keys()
                .any(|id| !snapshot.resources.contains_key(id))
            || self
                .authority
                .projects
                .keys()
                .any(|id| !snapshot.projects.contains_key(id))
        {
            return Err(fail(409, "authority_removal_requires_tombstone"));
        }
        for (id, next) in &snapshot.engagements {
            if let Some(old) = self.authority.engagements.get(id)
                && (old.server != next.server
                    || old.owner != next.owner
                    || next.registration_generation < old.registration_generation
                    || next.delegation_revision < old.delegation_revision
                    || ((old.coordinator != next.coordinator
                        || old.allow_self_approval != next.allow_self_approval
                        || old.delegation_expires_at_ms != next.delegation_expires_at_ms)
                        && next.delegation_revision == old.delegation_revision)
                    || (old.state == EngagementState::Revoked
                        && next.state != EngagementState::Revoked
                        && next.registration_generation == old.registration_generation))
            {
                return Err(fail(409, "authority_revision_conflict"));
            }
        }
        for (id, next) in &snapshot.resources {
            if let Some(old) = self.authority.resources.get(id)
                && (old.server_engagement_id != next.server_engagement_id
                    || next.revision < old.revision
                    || (next.revision == old.revision
                        && serde_json::to_value(next)? != serde_json::to_value(old)?))
            {
                return Err(fail(409, "authority_revision_conflict"));
            }
        }
        for (id, next) in &snapshot.projects {
            if let Some(old) = self.authority.projects.get(id)
                && (old.server_engagement_id != next.server_engagement_id
                    || old.owner != next.owner
                    || next.revision < old.revision
                    || (next.revision == old.revision && !project_progress(old, next)))
            {
                return Err(fail(409, "authority_revision_conflict"));
            }
        }
        self.authority = snapshot;
        Ok(())
    }
    pub fn load(state: &Value) -> Result<Self> {
        match state.get("rustWorkflows") {
            Some(v) => {
                serde_json::from_value(v.clone()).map_err(|_| fail(503, "workflow_state_invalid"))
            }
            None => Ok(Self::default()),
        }
    }
    pub fn save(&self, state: &mut Value) -> Result<()> {
        state["rustWorkflows"] = serde_json::to_value(self)?;
        Ok(())
    }
    fn engagement(&self, id: &ServerEngagementId) -> Result<&ServerEngagement> {
        self.authority
            .engagements
            .get(id.as_str())
            .ok_or_else(|| fail(404, "engagement_not_found"))
    }
    fn can_review(&self, request: &Request, actor: &MatrixUserId, now: u64) -> bool {
        self.engagement(request.engagement()).is_ok_and(|e| {
            e.state == EngagementState::Verified
                && e.coordinator_approval_v1
                && e.delegation_expires_at_ms > now
                && (actor == &e.coordinator
                    || matches!(request, Request::Agent(_) | Request::TokenTopUp(_))
                        && actor == &e.owner)
                && (e.allow_self_approval || actor != request.requester())
        })
    }
    fn visible(&self, action: &Action, actor: &MatrixUserId, now: u64) -> bool {
        action.request.owner() == actor
            || action.request.requester() == actor
            || self.can_review(&action.request, actor, now)
    }
    pub fn view(&self, id: &str, actor: &MatrixUserId, now: u64) -> Result<Value> {
        if let Some(association) = self.associations.get(id) {
            return self.association_view(association, actor, now);
        }
        let action = self
            .actions
            .get(id)
            .filter(|a| self.visible(a, actor, now))
            .ok_or_else(|| fail(404, "action_not_found"))?;
        let can_decide =
            action.state == "requested" && self.can_review(&action.request, actor, now);
        let mut result = serde_json::to_value(action)?;
        result["needsMyAction"] = json!(can_decide);
        result["canDecide"] = json!(can_decide);
        result["canContinue"] = json!(false);
        if let Some(receipt) = self
            .outbox
            .values()
            .find(|r| r["actionId"] == id && r["state"] == "refused")
        {
            result["failureReason"] = receipt["result"]["reason"].clone();
        }
        result["nextAction"] = if can_decide {
            json!("review")
        } else {
            Value::Null
        };
        result["ownerMxid"] = json!(action.request.owner());
        result["kind"] = result["request"]["kind"].clone();
        result["fleetId"] = json!(action.request.engagement());
        result["payload"] = self
            .definitions
            .get(&String::from(action.request.definition_digest().clone()))
            .cloned()
            .unwrap_or_else(|| json!({}));
        if result["payload"]["name"].as_str().is_none() {
            result["payload"]["name"] = result["payload"]["agentDefinition"]["name"]
                .as_str()
                .map(|s| json!(s))
                .unwrap_or_else(|| json!(action.request.id()));
        }
        if result["payload"]["reason"].as_str().is_none() {
            result["payload"]["reason"] = json!("");
        }
        if let Request::TokenTopUp(request) = &action.request {
            let agent_name = self
                .actions
                .values()
                .find_map(|a| {
                    let Request::Agent(agent) = &a.request else {
                        return None;
                    };
                    if agent.server_engagement_id != request.server_engagement_id
                        || agent.project_id != request.project_id
                        || self.observations.get(&a.id).is_none_or(|o| {
                            o["engagementId"] != request.agent_allocation_id.as_str()
                        })
                    {
                        return None;
                    }
                    let definition = self
                        .definitions
                        .get(&String::from(agent.definition_digest.clone()))?;
                    definition["agentDefinition"]["name"].as_str()
                })
                .unwrap_or(request.agent_allocation_id.as_str());
            result["payload"]["name"] = json!(format!(
                "More tokens · {}",
                agent_name.chars().take(160).collect::<String>()
            ));
            result["payload"]["reason"] = json!(format!(
                "Request {} additional tokens; current allocation {}.",
                u64::from(request.requested_additional_tokens),
                u64::from(request.expected_allocated_tokens)
            ));
        }
        if let Some(observation) = self.observations.get(id) {
            result["result"] = observation.clone();
            if action.execution == "ready" && !crate::updates::status_current(observation, now) {
                result["execution"] = json!("unknown");
            }
        }
        Ok(result)
    }
    pub fn list(
        &self,
        actor: &MatrixUserId,
        now: u64,
        view: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value> {
        if !["needs_action", "waiting", "history", "all"].contains(&view)
            || !(1..=100).contains(&limit)
        {
            return Err(fail(400, "invalid_page"));
        }
        let mut rows = Vec::new();
        let mut pending = 0;
        for action in self
            .actions
            .values()
            .filter(|a| self.visible(a, actor, now))
        {
            let row = self.view(&action.id, actor, now)?;
            let needs = row["needsMyAction"] == true;
            pending += usize::from(needs);
            if view == "all"
                || view == "needs_action" && needs
                || view == "waiting"
                    && !needs
                    && (action.state == "requested" || action.execution == "pending")
                || view == "history"
                    && (action.state == "rejected"
                        || matches!(
                            action.execution.as_str(),
                            "done" | "ended" | "allocation_refused"
                        ))
            {
                rows.push(row);
            }
        }
        for association in self.associations.values() {
            let row = match self.association_view(association, actor, now) {
                Ok(row) => row,
                Err(e) if e.status == 404 => continue,
                Err(e) => return Err(e),
            };
            let needs = row["needsMyAction"] == true;
            pending += usize::from(needs);
            if view == "all"
                || view == "needs_action" && needs
                || view == "waiting"
                    && !needs
                    && (association.state == "requested" || association.execution == "pending")
                || view == "history"
                    && (association.state == "rejected" || association.execution == "done")
            {
                rows.push(row);
            }
        }
        rows.sort_by(|a, b| {
            b["updatedAt"]
                .as_u64()
                .cmp(&a["updatedAt"].as_u64())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let total = rows.len();
        Ok(
            json!({"actions":rows.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":total,"pendingCount":pending,"room":null}),
        )
    }
    fn resource(
        &self,
        id: &ResourceAllocationId,
        engagement: &ServerEngagementId,
        manager: &MatrixUserId,
    ) -> Result<()> {
        let resource = self
            .authority
            .resources
            .get(id.as_str())
            .ok_or_else(|| fail(403, "resource_not_granted"))?;
        if &resource.server_engagement_id != engagement
            || u64::from(resource.allocated_tokens) == 0
            || !resource.eligible_managers.contains(manager)
        {
            return Err(fail(403, "resource_not_granted"));
        }
        Ok(())
    }
    pub fn submit(&mut self, request: Request, actor: &MatrixUserId, now: u64) -> Result<Value> {
        if request.requester() != actor || request.owner() != actor {
            return Err(fail(403, "project_owner_required"));
        }
        let e = self.engagement(request.engagement())?;
        if !actor.belongs_to(&e.server)
            || e.state != EngagementState::Verified
            || !e.coordinator_approval_v1
            || e.delegation_expires_at_ms <= now
        {
            return Err(fail(409, "engagement_unavailable"));
        }
        match &request {
            Request::Project(r) => {
                if r.resource_allocations.is_empty()
                    || r.resource_allocations.len() > 64
                    || r.resource_allocations.iter().collect::<BTreeSet<_>>().len()
                        != r.resource_allocations.len()
                {
                    return Err(fail(400, "invalid_resources"));
                }
                for resource in &r.resource_allocations {
                    self.resource(resource, &r.server_engagement_id, actor)?;
                }
            }
            Request::Agent(r) => {
                let p = self
                    .authority
                    .projects
                    .get(r.project_id.as_str())
                    .ok_or_else(|| fail(404, "project_not_found"))?;
                if p.state != ProjectState::Ready
                    || p.owner != *actor
                    || p.server_engagement_id != r.server_engagement_id
                    || p.revision != r.project_revision
                {
                    return Err(fail(409, "project_not_ready"));
                }
                if !p.resource_allocations.contains(&r.resource_allocation_id)
                    || u64::from(r.requested_tokens) == 0
                {
                    return Err(fail(403, "resource_not_granted"));
                }
                self.resource(&r.resource_allocation_id, &r.server_engagement_id, actor)?;
            }
            Request::TokenTopUp(r) => {
                let project = self
                    .authority
                    .projects
                    .get(r.project_id.as_str())
                    .ok_or_else(|| fail(404, "project_not_found"))?;
                if project.state != ProjectState::Ready
                    || project.owner != *actor
                    || project.server_engagement_id != r.server_engagement_id
                    || project.revision != r.project_revision
                    || !project
                        .resource_allocations
                        .contains(&r.resource_allocation_id)
                    || u64::from(r.requested_additional_tokens) == 0
                {
                    return Err(fail(409, "project_not_ready"));
                }
                self.resource(&r.resource_allocation_id, &r.server_engagement_id, actor)?;
                self.top_up_agent(r)?;
            }
        }
        let id = format!(
            "action_{}",
            &digest(&json!({"actor":actor,"requestId":request.id()}))?[..32]
        );
        if let Some(existing) = self.actions.get(&id) {
            if serde_json::to_value(&existing.request)? != serde_json::to_value(&request)? {
                return Err(fail(409, "idempotency_conflict"));
            }
            return self.view(&id, actor, now);
        }
        if self.actions.len() >= 10000
            || self
                .actions
                .values()
                .filter(|a| a.request.requester() == actor && a.state == "requested")
                .count()
                >= 100
        {
            return Err(fail(429, "inbox_full"));
        }
        self.actions.insert(
            id.clone(),
            Action {
                id: id.clone(),
                request,
                state: "requested".into(),
                revision: 1,
                created_at: now,
                updated_at: now,
                decision: None,
                execution: "pending".into(),
            },
        );
        self.notify(&id, now)?;
        self.view(&id, actor, now)
    }
    pub(crate) fn notify(&mut self, id: &str, now: u64) -> Result<()> {
        let a = &self.actions[id];
        let e = self.engagement(a.request.engagement())?;
        let mut recipients = vec![a.request.owner().clone(), e.coordinator.clone()];
        if matches!(a.request, Request::Agent(_) | Request::TokenTopUp(_)) {
            recipients.push(e.owner.clone());
        }
        for notice in self
            .notices
            .values_mut()
            .filter(|n| n["actionId"] == id && n["revision"] != a.revision)
        {
            notice["cancelled"] = json!(true);
        }
        for recipient in recipients {
            let key = format!(
                "{}_{}_{}",
                id,
                a.revision,
                &digest(&json!(recipient))?[..16]
            );
            self.notices.entry(key.clone()).or_insert_with(||json!({"id":key,"actionId":id,"revision":a.revision,"recipient":recipient,"createdAt":now,"dueAt":now,"attempt":0,"delivered":0,"seenAt":null,"cancelled":false}));
        }
        Ok(())
    }
    pub fn approve(
        &mut self,
        id: &str,
        command: Value,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let action = self
            .actions
            .get(id)
            .filter(|a| self.visible(a, actor, now))
            .cloned()
            .ok_or_else(|| fail(404, "action_not_found"))?;
        let engagement = self.engagement(action.request.engagement())?;
        let context = match &action.request {
            Request::Project(current) => {
                let c: ProjectApproval = serde_json::from_value(command.clone())?;
                authorize_project_approval(&c, current, engagement, actor, now)?;
                for r in &current.resource_allocations {
                    self.resource(r, &current.server_engagement_id, &current.owner)?;
                }
                c.context
            }
            Request::Agent(current) => {
                let c: AgentApproval = serde_json::from_value(command.clone())?;
                let project = self
                    .authority
                    .projects
                    .get(current.project_id.as_str())
                    .ok_or_else(|| fail(404, "project_not_found"))?;
                authorize_agent_approval(&c, current, engagement, project, actor, now)?;
                self.resource(
                    &current.resource_allocation_id,
                    &current.server_engagement_id,
                    &current.project_owner,
                )?;
                c.context
            }
            Request::TokenTopUp(current) => {
                let c: TokenTopUpApproval = serde_json::from_value(command.clone())?;
                let project = self
                    .authority
                    .projects
                    .get(current.project_id.as_str())
                    .ok_or_else(|| fail(404, "project_not_found"))?;
                authorize_token_top_up(&c, current, engagement, project, actor, now)?;
                self.resource(
                    &current.resource_allocation_id,
                    &current.server_engagement_id,
                    &current.project_owner,
                )?;
                self.top_up_agent(current)?;
                c.context
            }
        };
        let key = context.command_id.as_str().to_owned();
        let fingerprint = digest(&json!({"operation":"approve","actionId":id,"command":command}))?;
        if let Some(receipt) = self.receipts.get(&key) {
            if receipt["digest"] != fingerprint {
                return Err(fail(409, "command_conflict"));
            }
            return self.view(id, actor, now);
        }
        if action.state != "requested" {
            return Err(fail(409, "decision_conflict"));
        }
        let a = self.actions.get_mut(id).unwrap();
        a.state = "approved".into();
        a.revision += 1;
        a.updated_at = now;
        a.decision = Some(json!({"by":actor,"at":now,"commandId":key}));
        // Reservation and readiness can only come from Hagency's later receipt.
        self.outbox.insert(key.clone(),json!({"id":key,"actionId":id,"kind":"coordinator_approval","serverEngagementId":context.server_engagement_id,"command":command,"digest":fingerprint,"state":"pending","createdAt":now}));
        self.receipts.insert(
            key.clone(),
            json!({"commandId":key,"digest":fingerprint,"actionId":id}),
        );
        self.notify(id, now)?;
        self.view(id, actor, now)
    }

    /// Mini-app clients submit an intent, not an authority envelope. Bind its
    /// command to the current authenticated actor and the stored frozen request.
    pub fn approve_intent(
        &mut self,
        id: &str,
        expected_revision: u64,
        command_id: &str,
        reason: &str,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let _: CommandId = command_id.to_owned().try_into()?;
        if reason.len() > 2000 {
            return Err(fail(400, "reason_too_long"));
        }
        let fingerprint = digest(
            &json!({"actionId":id,"expectedRevision":expected_revision,"commandId":command_id,"actor":actor,"reason":reason,"decision":"approve"}),
        )?;
        if let Some(receipt) = self.receipts.get(command_id) {
            if receipt["intentDigest"] != fingerprint {
                return Err(fail(409, "command_conflict"));
            }
            return self.view(id, actor, now);
        }
        let action = self
            .actions
            .get(id)
            .filter(|a| self.visible(a, actor, now))
            .ok_or_else(|| fail(404, "action_not_found"))?;
        if action.revision != expected_revision || action.state != "requested" {
            return Err(fail(409, "decision_conflict"));
        }
        let engagement = self.engagement(action.request.engagement())?;
        let mut command = json!({"context":{"version":1,"commandId":command_id,"serverEngagementId":engagement.id,
            "registrationGeneration":engagement.registration_generation,"delegationRevision":engagement.delegation_revision,
            "actor":actor,"issuedAtMs":now,"expiresAtMs":engagement.delegation_expires_at_ms},
            "request":serde_json::to_value(&action.request)?["request"]});
        match &action.request {
            Request::Agent(request) => command["allocatedTokens"] = json!(request.requested_tokens),
            Request::TokenTopUp(request) => {
                command["additionalTokens"] = json!(request.requested_additional_tokens)
            }
            Request::Project(_) => {}
        }
        self.approve(id, command, actor, now)?;
        self.actions
            .get_mut(id)
            .ok_or_else(|| fail(503, "action_not_found"))?
            .decision
            .as_mut()
            .ok_or_else(|| fail(503, "decision_missing"))?["reason"] = json!(reason);
        self.receipts
            .get_mut(command_id)
            .ok_or_else(|| fail(503, "receipt_missing"))?["intentDigest"] = json!(fingerprint);
        self.view(id, actor, now)
    }
    pub fn reject(
        &mut self,
        id: &str,
        expected: u64,
        command_id: &str,
        reason: &str,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let _: CommandId = command_id.to_owned().try_into()?;
        if reason.trim().is_empty() || reason.len() > 1000 || reason.chars().any(char::is_control) {
            return Err(fail(400, "invalid_reason"));
        }
        let action = self
            .actions
            .get(id)
            .filter(|a| self.visible(a, actor, now))
            .ok_or_else(|| fail(404, "action_not_found"))?;
        if !self.can_review(&action.request, actor, now) {
            return Err(fail(403, "coordinator_required"));
        }
        let fingerprint = digest(
            &json!({"operation":"reject","actionId":id,"expectedRevision":expected,"commandId":command_id,"reason":reason,"actor":actor}),
        )?;
        if let Some(receipt) = self.receipts.get(command_id) {
            if receipt["digest"] != fingerprint {
                return Err(fail(409, "command_conflict"));
            }
            return self.view(id, actor, now);
        }
        if action.state != "requested" || action.revision != expected {
            return Err(fail(409, "decision_conflict"));
        }
        let a = self.actions.get_mut(id).unwrap();
        a.state = "rejected".into();
        a.revision += 1;
        a.updated_at = now;
        a.execution = "done".into();
        a.decision = Some(json!({"by":actor,"at":now,"commandId":command_id,"reason":reason}));
        self.receipts.insert(
            command_id.into(),
            json!({"commandId":command_id,"digest":fingerprint,"actionId":id}),
        );
        self.notify(id, now)?;
        self.view(id, actor, now)
    }

    fn top_up_agent(&self, request: &TokenTopUpRequest) -> Result<()> {
        let allowed = self.actions.values().any(|action| {
            let Request::Agent(agent) = &action.request else {
                return false;
            };
            let observation = self.observations.get(&action.id);
            agent.server_engagement_id == request.server_engagement_id
                && agent.project_id == request.project_id
                && agent.resource_allocation_id == request.resource_allocation_id
                && agent.project_owner == request.project_owner
                && action.state == "approved"
                && observation.is_some_and(|o| {
                    o["engagementId"] == request.agent_allocation_id.as_str()
                        && o["state"] == "active"
                        && o["allocatedTokens"] == json!(request.expected_allocated_tokens)
                })
        });
        if !allowed {
            return Err(fail(409, "agent_allocation_changed"));
        }
        Ok(())
    }
    pub fn seen(&mut self, id: &str, actor: &MatrixUserId, now: u64) -> Result<Value> {
        let result = self.view(id, actor, now)?;
        for n in self
            .notices
            .values_mut()
            .filter(|n| n["actionId"] == id && n["recipient"] == actor.as_str())
        {
            n["seenAt"] = json!(now);
        }
        Ok(result)
    }
    pub fn snooze(
        &mut self,
        id: &str,
        minutes: u64,
        actor: &MatrixUserId,
        now: u64,
    ) -> Result<Value> {
        let action = self.view(id, actor, now)?;
        if action["needsMyAction"] != true || !(1..=1440).contains(&minutes) {
            return Err(fail(400, "invalid_snooze"));
        }
        let until = now
            .checked_add(minutes * 60000)
            .ok_or_else(|| fail(400, "invalid_snooze"))?;
        for n in self.notices.values_mut().filter(|n| {
            n["actionId"] == id
                && n["recipient"] == actor.as_str()
                && n["revision"] == action["revision"]
        }) {
            n["dueAt"] = json!(until);
        }
        Ok(json!({"snoozedUntil":until,"action":action}))
    }
}

fn project_progress(old: &ProjectGrant, next: &ProjectGrant) -> bool {
    let mut compare = next.clone();
    compare.state = old.state;
    compare == *old
        && (old.state == next.state
            || matches!(
                (old.state, next.state),
                (
                    ProjectState::Approved,
                    ProjectState::Ready | ProjectState::Revoked
                ) | (ProjectState::Ready, ProjectState::Revoked)
            ))
}
