//! Mini-app form intents. Caller-supplied actor, room and authority fields are
//! rejected; frozen requests are derived from current server records.
use palpo_hagency_contract::{
    AgentRequest, MatrixUserId, ProjectRequest, ProjectState, RequestId, ResourceAllocationId,
    Tokens,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::App;
use crate::workflow::{Request, Workflows};
use crate::{Result, digest, fail, now_ms};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectIntent {
    kind: String,
    request_id: RequestId,
    fleet_id: String,
    resource_ids: Vec<ResourceAllocationId>,
    name: String,
    #[serde(default)]
    room_id: Option<String>,
    #[serde(default)]
    reason: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentDefinition {
    name: String,
    resource_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentIntent {
    request_id: RequestId,
    project_id: String,
    resource_allocation_id: ResourceAllocationId,
    role: String,
    requested_tokens: String,
    rate_per_day: Option<String>,
    agent_definition: AgentDefinition,
}

fn bounded(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && value.chars().count() <= max
        && !value.chars().any(char::is_control)
}
fn tokens(value: &str) -> Result<Tokens> {
    value
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == value)
        .ok_or_else(|| fail(400, "invalid_token_count"))?
        .try_into()
        .map_err(Into::into)
}
fn receipt(
    w: &Workflows,
    key: &str,
    fingerprint: &str,
    actor: &MatrixUserId,
) -> Result<Option<Value>> {
    if let Some(r) = w.submission_intents.get(key) {
        if r["digest"] != fingerprint {
            return Err(fail(409, "idempotency_conflict"));
        }
        return Ok(Some(w.view(
            r["actionId"].as_str().unwrap_or_default(),
            actor,
            now_ms(),
        )?));
    }
    Ok(None)
}
fn project_resources(
    state: &Value,
    w: &Workflows,
    actor: &MatrixUserId,
    input: &ProjectIntent,
) -> Result<()> {
    let catalog = crate::views::catalog(state, w, actor, now_ms(), json!({}))?;
    let offered: Vec<&str> = catalog["fleets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["id"] == input.fleet_id)
        .flat_map(|f| f["capabilities"]["offers"].as_array().into_iter().flatten())
        .flat_map(|o| o["resources"].as_array().into_iter().flatten())
        .filter_map(|r| r["allocationId"].as_str())
        .collect();
    if input.resource_ids.is_empty()
        || input.resource_ids.len() > 64
        || input
            .resource_ids
            .iter()
            .any(|id| !offered.contains(&id.as_str()))
        || input
            .resource_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != input.resource_ids.len()
    {
        return Err(fail(403, "resource_not_granted"));
    }
    Ok(())
}

impl App {
    fn plan(state: &mut Value, key: &str, plan: Value) -> Result<Value> {
        if !state["preparations"].is_object() {
            state["preparations"] = json!({});
        }
        if let Some(existing) = state["preparations"].get(key) {
            if existing["digest"] != plan["digest"] {
                return Err(fail(409, "idempotency_conflict"));
            }
            if existing["registrationGeneration"] != plan["registrationGeneration"]
                || existing["delegationRevision"] != plan["delegationRevision"]
            {
                return Err(fail(409, "workflow_binding_changed"));
            }
            return Ok(existing.clone());
        }
        if state["preparations"]
            .as_object()
            .is_some_and(|p| p.len() >= 10000)
        {
            return Err(fail(409, "preparation_limit"));
        }
        state["preparations"][key] = plan.clone();
        Ok(plan)
    }

    pub(crate) async fn create_project(&self, bearer: &str, input: Value) -> Result<Value> {
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let actor = &identity.user;
        let intent: ProjectIntent = serde_json::from_value(input.clone())?;
        if intent.kind != "project"
            || !bounded(&intent.name, 160)
            || intent.name.len() > 256
            || intent.reason.chars().count() > 2000
            || intent.reason.chars().any(char::is_control)
        {
            return Err(fail(400, "invalid_project"));
        }
        if let Some(room) = &intent.room_id {
            crate::rooms::selected_room_id(room, self.matrix.server().as_str())?;
        }
        let fingerprint = digest(&json!({"actor":actor,"intent":input}))?;
        let key = digest(&json!({"actor":actor,"requestId":intent.request_id}))?;
        let project_id = format!("project_{}", &key[..24]);
        let legacy_id = format!("action_{}", &key[..32]);
        let legacy_source;
        {
            let mut store = self.store.lock().await;
            let state = store.read()?;
            let w = Workflows::load(&state)?;
            if let Some(view) = receipt(&w, &key, &fingerprint, actor)? {
                return Ok(json!({"action":view}));
            }
            legacy_source = w.legacy_records.get(&legacy_id).cloned();
            if let Some(source) = &legacy_source
                && crate::legacy::continuation(&w, source, actor, now_ms()).as_ref() != Some(&input) {
                return Err(fail(409, "legacy_continuation_mismatch"));
            }
            project_resources(&state, &w, actor, &intent)?;
            let e = &w.authority.engagements[&intent.fleet_id];
            store.transaction(|state|Self::plan(state,&key,json!({"digest":fingerprint,"input":input,"actor":actor,
                "fleetId":intent.fleet_id,"projectId":project_id,"registrationGeneration":e.registration_generation,"delegationRevision":e.delegation_revision})))?;
        }
        let room = if let Some(room) = &intent.room_id {
            self.attach_room(&key, room, &session.token, actor.as_str()).await?
        } else {
            self.prepare_room(&key, "project", &session.token, actor.as_str()).await?
        };
        let dm = self
            .prepare_room(&key, "approvals", &session.token, actor.as_str())
            .await?;
        // Revalidate borrowed identity after all potentially slow Matrix calls.
        self.authenticate(bearer).await?;
        let mut definition = json!({"name":intent.name,"roomId":room,"ownerDmRoomId":dm});
        if !intent.reason.is_empty() {
            definition["reason"] = json!(intent.reason);
        }
        self.store.lock().await.transaction(|state| {
            let mut w = Workflows::load(state)?;
            if legacy_source != w.legacy_records.get(&legacy_id).cloned() {
                return Err(fail(409, "legacy_source_changed"));
            }
            project_resources(state, &w, actor, &intent)?;
            check_plan(state, &w, &key, &intent.fleet_id)?;
            let request = ProjectRequest {
                id: intent.request_id,
                revision: 1.try_into()?,
                server_engagement_id: intent.fleet_id.try_into()?,
                project_id: project_id.try_into()?,
                owner: actor.clone(),
                requester: actor.clone(),
                definition_digest: digest(&definition)?.try_into()?,
                resource_allocations: intent.resource_ids,
            };
            let mut view =
                w.submit_definition(Request::Project(request), definition, actor, now_ms())?;
            if let Some(source) = &legacy_source {
                w.legacy_sources.insert(legacy_id.clone(),json!({"source":"actionInbox","sourceId":legacy_id,
                    "digest":digest(source)?,"originalAction":source,"continuedAtMs":now_ms()}));
                w.legacy_records.remove(&legacy_id);
                view = w.view(&legacy_id, actor, now_ms())?;
            }
            finish(
                state,
                &mut w,
                &key,
                &fingerprint,
                &view,
                actor,
                "project.submitted",
            )?;
            Ok(json!({"action":view}))
        })
    }

    pub(crate) async fn create_agent(&self, bearer: &str, input: Value) -> Result<Value> {
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let actor = &identity.user;
        let intent: AgentIntent = serde_json::from_value(input.clone())?;
        let name = &intent.agent_definition.name;
        if !bounded(name, 64)
            || !name.chars().next().is_some_and(char::is_alphabetic)
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            || !bounded(&intent.role, 80)
        {
            return Err(fail(400, "invalid_agent_definition"));
        }
        let amount = tokens(&intent.requested_tokens)?;
        let rate = intent
            .rate_per_day
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(tokens)
            .transpose()?;
        let fingerprint = digest(&json!({"actor":actor,"intent":input}))?;
        let key = digest(&json!({"actor":actor,"requestId":intent.request_id}))?;
        let (mut definition, project, as_token, representative) = {
            let mut store = self.store.lock().await;
            let state = store.read()?;
            let w = Workflows::load(&state)?;
            if let Some(view) = receipt(&w, &key, &fingerprint, actor)? {
                return Ok(json!({"action":view,"request":view}));
            }
            let project = agent_project(&state, &w, actor, &intent, amount)?;
            let fleet_id = project.server_engagement_id.as_str();
            let e = &w.authority.engagements[fleet_id];
            let fleet = &state["fleets"][fleet_id];
            let project_definition = w
                .actions
                .values()
                .find_map(|a| match &a.request {
                    Request::Project(r)
                        if r.project_id == project.project_id && r.revision == project.revision =>
                    {
                        w.definitions
                            .get(&String::from(r.definition_digest.clone()))
                    }
                    _ => None,
                })
                .ok_or_else(|| fail(409, "project_rooms_unavailable"))?;
            let payload = json!({"v":1,"fleetId":fleet_id,"requestId":intent.request_id,"requesterMxid":actor,
                "sourceRoomId":fleet["receptionRoomId"],"targetProjectId":project.project_id,"targetRoomId":project_definition["roomId"],
                "ownerMxid":actor,"ownerDmRoomId":project_definition["ownerDmRoomId"],"role":intent.role,"requestedTokens":amount,"ratePerDay":rate,
                "authVersion":1,"agentDefinition":{"name":name,"resourceId":intent.agent_definition.resource_id}});
            for field in ["sourceRoomId", "targetRoomId", "ownerDmRoomId"] {
                if payload[field].as_str().is_none_or(|r| !r.starts_with('!')) {
                    return Err(fail(409, "project_rooms_unavailable"));
                }
            }
            let plan=store.transaction(|state|Self::plan(state,&key,json!({"digest":fingerprint,"input":input,"actor":actor,
                "fleetId":fleet_id,"projectId":project.project_id,"projectRevision":project.revision,"definition":payload,
                "registrationGeneration":e.registration_generation,"delegationRevision":e.delegation_revision})))?;
            if plan["definition"] != payload || plan["projectRevision"] != json!(project.revision) {
                return Err(fail(409, "workflow_binding_changed"));
            }
            let token = fleet["registration"]["as_token"]
                .as_str()
                .ok_or_else(|| fail(409, "registration_unavailable"))?
                .to_owned();
            let rep = fleet["representativeMxid"]
                .as_str()
                .ok_or_else(|| fail(409, "registration_unavailable"))?
                .to_owned();
            (payload, project, token, rep)
        };
        let room = definition["sourceRoomId"].as_str().unwrap().to_owned();
        for (field, purpose) in [("targetRoomId", "project"), ("ownerDmRoomId", "approvals")] {
            let binding = json!({"v":1,"fleetId":project.server_engagement_id,"purpose":purpose,"projectId":project.project_id,"ownerMxid":actor,"authVersion":1});
            let events = self
                .matrix
                .room_state(
                    definition[field].as_str().unwrap(),
                    &session.token,
                    actor.as_str(),
                )
                .await?;
            crate::rooms::validate(&events, &binding, actor.as_str(), purpose == "approvals")?;
        }
        // The runtime representative invites the authenticated manager into its
        // reception; the user's own credential accepts and signs the request.
        let members = self
            .matrix
            .room_state(&room, &as_token, &representative)
            .await?;
        let joined = members.as_array().is_some_and(|events| {
            events.iter().any(|e| {
                e["type"] == "m.room.member"
                    && e["state_key"] == actor.as_str()
                    && e["content"]["membership"] == "join"
            })
        });
        if !joined {
            self.matrix
                .segments(
                    Method::POST,
                    &["_matrix", "client", "v3", "rooms", &room, "invite"],
                    &as_token,
                    Some(&representative),
                    Some(&json!({"user_id":actor})),
                )
                .await?;
            self.matrix
                .segments(
                    Method::POST,
                    &["_matrix", "client", "v3", "join", &room],
                    &session.token,
                    None,
                    Some(&json!({})),
                )
                .await?;
        }
        let mut public = definition.clone();
        public.as_object_mut().unwrap().remove("ownerDmRoomId");
        let event = self
            .source_event(&key, &room, &session.token, actor, &public)
            .await?;
        definition["sourceEventId"] = json!(event);
        self.authenticate(bearer).await?;
        self.store.lock().await.transaction(|state| {
            let mut w = Workflows::load(state)?;
            let current = agent_project(state, &w, actor, &intent, amount)?;
            if current != project {
                return Err(fail(409, "workflow_binding_changed"));
            }
            check_plan(state, &w, &key, project.server_engagement_id.as_str())?;
            let request = AgentRequest {
                id: intent.request_id,
                revision: 1.try_into()?,
                server_engagement_id: project.server_engagement_id,
                project_id: project.project_id,
                project_revision: project.revision,
                resource_allocation_id: intent.resource_allocation_id,
                project_owner: actor.clone(),
                requester: actor.clone(),
                definition_digest: digest(&definition)?.try_into()?,
                requested_tokens: amount,
            };
            let view = w.submit_definition(Request::Agent(request), definition, actor, now_ms())?;
            finish(
                state,
                &mut w,
                &key,
                &fingerprint,
                &view,
                actor,
                "agent.submitted",
            )?;
            Ok(json!({"action":view,"request":view}))
        })
    }
}

impl App {
    async fn source_event(
        &self,
        key: &str,
        room: &str,
        token: &str,
        actor: &MatrixUserId,
        content: &Value,
    ) -> Result<String> {
        let plan = self.store.lock().await.read()?["preparations"][key].clone();
        if let Some(id) = plan["sourceEventId"].as_str() {
            return Ok(id.to_owned());
        }
        let token_key = digest(&json!(token))?;
        let mut recovered = None;
        if plan["eventAttempted"] == true && plan["eventTokenDigest"] != token_key {
            // Matrix transaction IDs are scoped to an access token. After a new
            // login, reconcile back to the persisted pre-send boundary before
            // considering another send, instead of silently duplicating events.
            let mut cursor = None;
            let mut reached = false;
            for _ in 0..10 {
                let page = self
                    .matrix
                    .messages(room, token, cursor.as_deref(), 100)
                    .await?;
                let events = page["chunk"]
                    .as_array()
                    .filter(|e| e.len() <= 100)
                    .ok_or_else(|| fail(502, "invalid_matrix_history"))?;
                for event in events {
                    if !plan["eventBoundary"].is_null()
                        && event["event_id"] == plan["eventBoundary"]
                    {
                        reached = true;
                        break;
                    }
                    if event["sender"] == actor.as_str()
                        && event["type"] == "com.hagency.engagement.request.v1"
                        && event["content"]["requestId"] == content["requestId"]
                    {
                        if event["content"] != *content || recovered.is_some() {
                            return Err(fail(409, "source_event_conflict"));
                        }
                        recovered = Some(event_id(event)?);
                    }
                }
                if reached || page["end"].is_null() {
                    reached = true;
                    break;
                }
                let next = page["end"]
                    .as_str()
                    .ok_or_else(|| fail(502, "invalid_matrix_history"))?
                    .to_owned();
                if cursor.as_ref() == Some(&next) {
                    return Err(fail(409, "source_event_reconciliation_required"));
                }
                cursor = Some(next);
            }
            if !reached {
                return Err(fail(409, "source_event_reconciliation_required"));
            }
        }
        let id = if let Some(id) = recovered {
            id
        } else {
            if plan["eventAttempted"] != true {
                let history = self.matrix.messages(room, token, None, 1).await?;
                let events = history["chunk"]
                    .as_array()
                    .filter(|e| e.len() <= 1)
                    .ok_or_else(|| fail(502, "invalid_matrix_history"))?;
                let boundary = events.first().map(event_id).transpose()?;
                self.store.lock().await.transaction(|state| {
                    state["preparations"][key]["eventBoundary"] = json!(boundary);
                    state["preparations"][key]["eventAttempted"] = json!(true);
                    state["preparations"][key]["eventTokenDigest"] = json!(token_key);
                    Ok(())
                })?;
            }
            let event = self
                .matrix
                .segments(
                    Method::PUT,
                    &[
                        "_matrix",
                        "client",
                        "v3",
                        "rooms",
                        room,
                        "send",
                        "com.hagency.engagement.request.v1",
                        &format!("palpo_{key}"),
                    ],
                    token,
                    None,
                    Some(content),
                )
                .await?;
            event_id(&event)?
        };
        self.store.lock().await.transaction(|state| {
            state["preparations"][key]["sourceEventId"] = json!(id);
            Ok(())
        })?;
        Ok(id)
    }
}
fn event_id(event: &Value) -> Result<String> {
    event["event_id"]
        .as_str()
        .filter(|e| {
            e.starts_with('$')
                && e.len() <= 255
                && !e.chars().any(|c| c.is_whitespace() || c.is_control())
        })
        .map(str::to_owned)
        .ok_or_else(|| fail(502, "invalid_source_event"))
}

fn check_plan(state: &Value, w: &Workflows, key: &str, fleet: &str) -> Result<()> {
    let e = w
        .authority
        .engagements
        .get(fleet)
        .ok_or_else(|| fail(409, "engagement_unavailable"))?;
    let plan = &state["preparations"][key];
    if plan["registrationGeneration"] != json!(e.registration_generation)
        || plan["delegationRevision"] != json!(e.delegation_revision)
    {
        return Err(fail(409, "workflow_binding_changed"));
    }
    Ok(())
}
fn finish(
    state: &mut Value,
    w: &mut Workflows,
    key: &str,
    fingerprint: &str,
    view: &Value,
    actor: &MatrixUserId,
    event: &str,
) -> Result<()> {
    w.submission_intents.insert(
        key.into(),
        json!({"digest":fingerprint,"actionId":view["id"]}),
    );
    w.save(state)?;
    state["audit"].as_array_mut().ok_or_else(||fail(503,"workflow_state_invalid"))?
        .push(json!({"atMs":now_ms(),"actor":actor,"action":event,"actionId":view["id"],"result":"committed"}));
    Ok(())
}
fn agent_project(
    state: &Value,
    w: &Workflows,
    actor: &MatrixUserId,
    intent: &AgentIntent,
    amount: Tokens,
) -> Result<palpo_hagency_contract::ProjectGrant> {
    let project = w
        .authority
        .projects
        .get(&intent.project_id)
        .filter(|p| p.owner == *actor && p.state == ProjectState::Ready)
        .ok_or_else(|| fail(403, "project_owner_required"))?;
    let catalog = crate::views::catalog(
        state,
        w,
        actor,
        now_ms(),
        json!({"projectId":intent.project_id}),
    )?;
    let available = catalog["fleets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| f["capabilities"]["offers"].as_array().into_iter().flatten())
        .filter(|o| o["role"] == intent.role)
        .flat_map(|o| o["resources"].as_array().into_iter().flatten())
        .any(|r| {
            r["id"] == intent.agent_definition.resource_id
                && r["allocationId"] == intent.resource_allocation_id.as_str()
                && r["allocatedTokens"]
                    .as_u64()
                    .is_some_and(|n| n >= u64::from(amount))
        });
    if !available {
        return Err(fail(403, "resource_not_granted"));
    }
    let collision=w.actions.values().any(|a|matches!(&a.request,Request::Agent(r) if r.project_id==project.project_id && r.id!=intent.request_id)
        && a.state!="rejected" && a.execution!="retired" && w.definitions.get(&String::from(a.request.definition_digest().clone()))
            .is_some_and(|d|d["agentDefinition"]["name"].as_str().is_some_and(|n|n.to_lowercase()==intent.agent_definition.name.to_lowercase())));
    if collision {
        return Err(fail(409, "agent_name_in_use"));
    }
    Ok(project.clone())
}
