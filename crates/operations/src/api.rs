use std::collections::BTreeMap;
use std::sync::Arc;

use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::matrix::{Identity, Matrix};
use crate::store::Store;
use crate::workflow::{Request as WorkflowRequest, Workflows};
use crate::{Result, digest, fail, now_ms, secret};

pub const APP_ID: &str = "im.palpo.operations";
pub const SERVICES: &[&str] = &[
    "palpo.intent.new",
    "palpo.session.open",
    "palpo.session.disconnect",
    "palpo.inbox.list",
    "palpo.inbox.get",
    "palpo.inbox.submit",
    "palpo.inbox.decide",
    "palpo.inbox.seen",
    "palpo.inbox.snooze",
    "palpo.projects.list",
    "palpo.requests.list",
    "palpo.catalog.list",
    "palpo.requests.create",
    "palpo.fleets.install",
    "palpo.fleets.export",
    "palpo.fleets.list",
    "palpo.fleets.connect",
    "palpo.agents.control",
    "palpo.notifications.get",
    "palpo.notifications.set",
    "palpo.actions.room.get",
    "palpo.actions.room.ensure",
    "palpo.requests.open",
    "palpo.accounts.list",
    "palpo.accounts.open",
];

// Names in the reviewed existing app contract. During migration the native
// host may request its complete manifest; only implemented services are granted.
const PENDING_SERVICES: &[&str] = &[
    "palpo.fleets.register",
    "palpo.fleets.set_state",
    "palpo.fleets.migrate",
    "palpo.fleets.queue",
    "palpo.agents.list",
    "palpo.agents.register",
    "palpo.agents.rename",
    "palpo.agents.retire",
    "palpo.activity.list",
    "palpo.inbox.activate",
];

#[derive(Clone)]
pub(crate) struct Session {
    pub(crate) token: String,
    identity: Identity,
    services: Vec<String>,
    expires_at: u64,
}

pub struct App {
    pub matrix: Matrix,
    pub store: Mutex<Store>,
    sessions: Mutex<BTreeMap<String, Session>>,
    pub(crate) mutation: Mutex<()>,
    pub(crate) host: String,
    pub(crate) public_origin: String,
    pub(crate) transport_origin: Option<String>,
    pub(crate) relay_origin: Option<String>,
    pub(crate) association_admin: Option<palpo_hagency_contract::MatrixUserId>,
    pub(crate) transport_host: Option<String>,
    pub(crate) relay_host: Option<String>,
    pub(crate) notifications: Option<crate::notifications::Configuration>,
    pub(crate) accounts: Option<crate::accounts::Configuration>,
    pub(crate) account_limits: Mutex<crate::accounts::Limits>,
    ttl_ms: u64,
}

impl App {
    pub fn new(
        matrix: Matrix,
        mut store: Store,
        public_origin: &str,
        ttl_ms: u64,
    ) -> Result<Arc<Self>> {
        let origin =
            reqwest::Url::parse(public_origin).map_err(|_| fail(400, "invalid_public_origin"))?;
        if !(origin.scheme() == "https"
            || origin.scheme() == "http"
                && matches!(origin.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || ttl_ms == 0
            || ttl_ms > 900000
        {
            return Err(fail(400, "invalid_public_origin"));
        }
        let host = match origin.port() {
            Some(port) => format!("{}:{port}", origin.host_str().unwrap_or_default()),
            None => origin.host_str().unwrap_or_default().to_owned(),
        };
        store.bind(matrix.server().as_str(), &matrix.origin())?;
        Ok(Arc::new(Self {
            matrix,
            store: Mutex::new(store),
            sessions: Mutex::new(BTreeMap::new()),
            mutation: Mutex::new(()),
            host,
            public_origin: origin.origin().ascii_serialization(),
            transport_origin: None,
            relay_origin: None,
            association_admin: None,
            transport_host: None,
            relay_host: None,
            notifications: None,
            accounts: None,
            account_limits: Mutex::new(crate::accounts::Limits::default()),
            ttl_ms,
        }))
    }

    pub fn with_transport(mut self: Arc<Self>, transport: &str, relay: &str) -> Result<Arc<Self>> {
        let host = |origin: &str| -> Result<String> {
            let checked = Matrix::new(origin, self.matrix.server().clone())?;
            let url = reqwest::Url::parse(&checked.origin())
                .map_err(|_| fail(400, "invalid_transport_origin"))?;
            Ok(match url.port() {
                Some(p) => format!("{}:{p}", url.host_str().unwrap_or_default()),
                None => url.host_str().unwrap_or_default().to_owned(),
            })
        };
        let transport_host = host(transport)?;
        let relay_host = host(relay)?;
        let app =
            Arc::get_mut(&mut self).ok_or_else(|| fail(409, "transport_configuration_locked"))?;
        app.transport_host = Some(transport_host);
        app.relay_host = Some(relay_host);
        app.transport_origin = Some(transport.trim_end_matches('/').into());
        app.relay_origin = Some(relay.trim_end_matches('/').into());
        Ok(self)
    }

    pub fn with_association_admin(
        mut self: Arc<Self>,
        admin: palpo_hagency_contract::MatrixUserId,
    ) -> Result<Arc<Self>> {
        if !admin.belongs_to(self.matrix.server()) {
            return Err(fail(400, "local_admin_required"));
        }
        Arc::get_mut(&mut self)
            .ok_or_else(|| fail(409, "association_configuration_locked"))?
            .association_admin = Some(admin);
        Ok(self)
    }

    pub fn with_notifications(
        mut self: Arc<Self>,
        config: crate::notifications::Configuration,
    ) -> Result<Arc<Self>> {
        config.validate(&self)?;
        Arc::get_mut(&mut self)
            .ok_or_else(|| fail(409, "notification_configuration_locked"))?
            .notifications = Some(config);
        Ok(self)
    }

    async fn identity(&self, session: &Session, identity: &Identity) -> Result<Value> {
        let workflows = Workflows::load(&self.store.lock().await.read()?)?;
        let can_approve = workflows.authority.engagements.values().any(|e| {
            e.coordinator == identity.user
                && e.coordinator_approval_v1
                && e.state == palpo_hagency_contract::EngagementState::Verified
                && e.delegation_expires_at_ms > now_ms()
        });
        Ok(
            json!({"version":1,"userId":identity.user,"isAdmin":identity.admin,"canApproveProjects":can_approve,
            "isResourceOwner":workflows.authority.engagements.values().any(|e|e.owner==identity.user),
            "serverName":self.matrix.server(),"services":session.services,"callbackOrigins":[],"outboundAvailable":self.transport_origin.is_some(),
            "features":{"inbox":true,"contributions":false,"projectApproval":can_approve,"remoteAgentDecisions":false,"topUps":true,
                "rustWorkflowRequests":1,"coordinatorTransport":1,"runtimeExecution":self.transport_host.is_some(),"matrixNotifications":self.notifications.is_some(),"associations":self.association_admin.is_some()}}),
        )
    }

    async fn open(&self, token: String, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Open {
            app_id: String,
            bundle_digest: String,
            services: Vec<String>,
        }
        let input: Open = serde_json::from_value(input)?;
        let _: palpo_hagency_contract::DefinitionDigest = input.bundle_digest.try_into()?;
        if input.app_id != APP_ID
            || input.services.is_empty()
            || input.services.len() > SERVICES.len() + PENDING_SERVICES.len()
            || input
                .services
                .iter()
                .any(|s| !SERVICES.contains(&s.as_str()) && !PENDING_SERVICES.contains(&s.as_str()))
            || input
                .services
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != input.services.len()
        {
            return Err(fail(403, "app_not_supported"));
        }
        let identity = self.matrix.authenticate(&token).await?;
        let now = now_ms();
        let bearer = secret();
        let key = digest(&json!(bearer))?;
        let session = Session {
            token,
            identity,
            services: input
                .services
                .into_iter()
                .filter(|s| SERVICES.contains(&s.as_str()))
                .collect(),
            expires_at: now + self.ttl_ms,
        };
        // Build the response before insertion so a failed store read cannot leak
        // an unusable session slot.
        let mut response = self.identity(&session, &session.identity).await?;
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| s.expires_at > now);
        if sessions.len() >= 512 {
            return Err(fail(503, "sessions_busy"));
        }
        response["sessionToken"] = json!(bearer);
        response["expiresAt"] = json!(session.expires_at);
        sessions.insert(key, session);
        Ok(response)
    }

    pub(crate) async fn authenticate(&self, bearer: &str) -> Result<(String, Session, Identity)> {
        let key = digest(&json!(bearer))?;
        let session = self
            .sessions
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| fail(401, "app_session_expired"))?;
        if session.expires_at <= now_ms() {
            self.sessions.lock().await.remove(&key);
            return Err(fail(401, "app_session_expired"));
        }
        let identity = match self.matrix.authenticate(&session.token).await {
            Ok(i) => i,
            Err(e) => {
                if e.status == 401 || e.status == 403 {
                    self.sessions.lock().await.remove(&key);
                }
                return Err(e);
            }
        };
        if identity.user != session.identity.user {
            self.sessions.lock().await.remove(&key);
            return Err(fail(403, "identity_changed"));
        }
        if session.expires_at <= now_ms() || !self.sessions.lock().await.contains_key(&key) {
            return Err(fail(401, "app_session_expired"));
        }
        Ok((key, session, identity))
    }

    async fn disconnect(&self, bearer: &str) -> Result<Value> {
        let _queue = self.mutation.lock().await;
        let (key, ..) = self.authenticate(bearer).await?;
        self.sessions.lock().await.remove(&key);
        // Never log the Matrix user out when disconnecting its mini app.
        Ok(json!({"disconnected":true}))
    }

    async fn call(&self, bearer: &str, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Call {
            service: String,
            #[serde(default = "empty")]
            args: Value,
        }
        fn empty() -> Value {
            json!({})
        }
        let input: Call = serde_json::from_value(input)?;
        let (_, session, identity) = self.authenticate(bearer).await?;
        if !session.services.contains(&input.service) {
            return Err(fail(403, "service_not_granted"));
        }
        let actor = &identity.user;
        if input.service == "palpo.inbox.decide"
            && input.args["id"].as_str().is_some_and(|id| !id.is_empty())
        {
            let w = Workflows::load(&self.store.lock().await.read()?)?;
            if w.associations
                .contains_key(input.args["id"].as_str().unwrap_or_default())
            {
                return self.decide_association(bearer, input.args).await;
            }
        }
        match input.service.as_str() {
            "palpo.accounts.list" | "palpo.accounts.open" => {
                self.account_service(bearer, &input.service, input.args)
                    .await
            }
            "palpo.requests.open" => self.open_agent_chat(bearer, input.args).await,
            "palpo.actions.room.get" | "palpo.actions.room.ensure" => {
                self.actions_room(bearer, &input.service, input.args).await
            }
            "palpo.fleets.list" => self.list_fleets(bearer, input.args).await,
            "palpo.fleets.connect" => self.connect_fleet(bearer, input.args).await,
            "palpo.fleets.install" | "palpo.fleets.export" => {
                self.association_fleet(bearer, &input.service, input.args)
                    .await
            }
            "palpo.intent.new" => {
                empty_args(&input.args)?;
                Ok(json!({"requestId":&secret()[..40]}))
            }
            "palpo.notifications.get" => {
                empty_args(&input.args)?;
                crate::preferences::get(&self.store.lock().await.read()?, actor)
            }
            "palpo.session.open" => {
                empty_args(&input.args)?;
                self.identity(&session, &identity).await
            }
            "palpo.session.disconnect" => {
                empty_args(&input.args)?;
                self.disconnect(bearer).await
            }
            "palpo.inbox.list" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Page {
                    #[serde(default = "view")]
                    view: String,
                    #[serde(default)]
                    offset: usize,
                    #[serde(default = "limit")]
                    limit: usize,
                }
                fn view() -> String {
                    "needs_action".into()
                }
                fn limit() -> usize {
                    50
                }
                let p: Page = serde_json::from_value(input.args)?;
                Workflows::load(&self.store.lock().await.read()?)?.list(
                    actor,
                    now_ms(),
                    &p.view,
                    p.offset,
                    p.limit,
                )
            }
            "palpo.inbox.get" => {
                let id: Id = serde_json::from_value(input.args)?;
                Ok(
                    json!({"action":Workflows::load(&self.store.lock().await.read()?)?.view(&id.id,actor,now_ms())?}),
                )
            }
            "palpo.projects.list" | "palpo.requests.list" => {
                let page = serde_json::from_value(input.args)?;
                let state = self.store.lock().await.read()?;
                let workflows = Workflows::load(&state)?;
                if input.service == "palpo.projects.list" {
                    crate::views::projects(&state, &workflows, actor, now_ms(), page)
                } else {
                    crate::views::agents(&state, &workflows, actor, now_ms(), page)
                }
            }
            "palpo.catalog.list" => {
                let state = self.store.lock().await.read()?;
                crate::views::catalog(
                    &state,
                    &Workflows::load(&state)?,
                    actor,
                    now_ms(),
                    input.args,
                )
            }
            "palpo.requests.create" => self.create_agent(bearer, input.args).await,
            "palpo.inbox.submit"
                if input.args["kind"] == "project" && input.args.get("fleetId").is_some() =>
            {
                self.create_project(bearer, input.args).await
            }
            service => {
                // Recheck borrowed identity and expiry after waiting in the same
                // mutation queue as other API callers.
                let _queue = self.mutation.lock().await;
                let (_, _, identity) = self.authenticate(bearer).await?;
                let actor = &identity.user;
                let now = now_ms();
                self.store.lock().await.transaction_sql(|state,tx| {
                    let mut workflows = Workflows::load(state)?;
                    let before = serde_json::to_value(&workflows)?;
                    let result = match service {
                        "palpo.notifications.set" => crate::preferences::set(state,&mut workflows,input.args.clone(),actor,now)?,
                        "palpo.agents.control" => json!({"action":crate::lifecycle::submit(&mut workflows,state,tx,input.args.clone(),actor,now)?}),
                        "palpo.inbox.submit" => {
                            if input.args["kind"] == "token_top_up" && input.args.get("agentActionId").is_some() {
                                json!({"action":crate::intents::top_up(&mut workflows,state,input.args.clone(),actor,now)?})
                            } else {
                                let mut args=input.args.clone();
                                let definition=args.as_object_mut().and_then(|o|o.remove("definition"));
                                let request: WorkflowRequest =serde_json::from_value(args)?;
                                json!({"action":match definition {Some(definition)=>workflows.submit_definition(request,definition,actor,now)?,None=>workflows.submit(request,actor,now)?}})
                            }
                        }
                        "palpo.inbox.decide" => {
                            #[derive(Deserialize)]
                            #[serde(rename_all = "camelCase", deny_unknown_fields)]
                            struct Decision {
                                id: String,
                                decision: String,
                                command: Option<Value>,
                                expected_revision: Option<u64>,
                                command_id: Option<String>,
                                reason: Option<String>,
                            }
                            let decision: Decision = serde_json::from_value(input.args.clone())?;
                            let result = match decision.decision.as_str() {
                                "approve" if decision.command.is_none() => workflows.approve_intent(
                                    &decision.id,
                                    decision.expected_revision.ok_or_else(||fail(400,"revision_required"))?,
                                    &decision.command_id.ok_or_else(||fail(400,"command_id_required"))?,
                                    decision.reason.as_deref().unwrap_or_default(),
                                    actor,
                                    now,
                                )?,
                                "approve"
                                    if decision.expected_revision.is_none()
                                        && decision.command_id.is_none()
                                        && decision.reason.is_none() =>
                                {
                                    workflows.approve(
                                        &decision.id,
                                        decision.command.ok_or_else(|| {
                                            fail(400, "approval_command_required")
                                        })?,
                                        actor,
                                        now,
                                    )?
                                }
                                "reject" if decision.command.is_none() => workflows.reject(
                                    &decision.id,
                                    decision
                                        .expected_revision
                                        .ok_or_else(|| fail(400, "revision_required"))?,
                                    &decision
                                        .command_id
                                        .ok_or_else(|| fail(400, "command_id_required"))?,
                                    &decision
                                        .reason
                                        .ok_or_else(|| fail(400, "reason_required"))?,
                                    actor,
                                    now,
                                )?,
                                _ => return Err(fail(400, "invalid_decision")),
                            };
                            json!({"action":result})
                        }
                        "palpo.inbox.seen" => {
                            let id: Id = serde_json::from_value(input.args.clone())?;
                            json!({"action":workflows.seen(&id.id,actor,now)?})
                        }
                        "palpo.inbox.snooze" => {
                            #[derive(Deserialize)]
                            #[serde(deny_unknown_fields)]
                            struct Snooze {
                                id: String,
                                minutes: u64,
                            }
                            let s: Snooze = serde_json::from_value(input.args.clone())?;
                            workflows.snooze(&s.id, s.minutes, actor, now)?
                        }
                        _ => return Err(fail(501, "service_not_implemented")),
                    };
                    workflows.enqueue_commands(state,tx)?;
                    if serde_json::to_value(&workflows)? != before {
                        workflows.save(state)?;
                        if !state["audit"].is_array() {
                            return Err(fail(503, "workflow_state_invalid"));
                        }
                        state["audit"].as_array_mut().unwrap().push(
                            json!({"atMs":now,"actor":actor,"action":service,"result":"committed"}),
                        );
                    }
                    Ok(result)
                })
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
fn empty_args(args: &Value) -> Result<()> {
    if args != &json!({}) {
        Err(fail(400, "invalid_arguments"))
    } else {
        Ok(())
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .hoop(affix_state::inject(app))
        .push(Router::with_path("healthz").get(health))
        .push(crate::machine::router())
        .push(crate::accounts::router())
        .push(
            Router::with_path("_palpo/miniapp/v1/{operation}")
                .hoop(salvo::size_limiter::max_size(16384))
                .post(dispatch),
        )
}

#[handler]
async fn health() -> &'static str {
    "ok"
}

#[handler]
async fn dispatch(req: &mut salvo::Request, depot: &mut Depot, res: &mut Response) {
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    let result = async {
        let app = depot
            .get_typed::<Arc<App>>()
            .map_err(|_| fail(503, "service_unavailable"))?;
        if req.headers().contains_key("origin") || req.headers().contains_key("cookie") {
            return Err(fail(403, "host_only"));
        }
        let host = req
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if host != app.host {
            return Err(fail(403, "host_forbidden"));
        }
        if req
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or_default())
            != Some("application/json")
        {
            return Err(fail(415, "json_required"));
        }
        let headers = req.headers().get_all("authorization");
        if headers.iter().count() != 1 {
            return Err(fail(401, "bearer_required"));
        }
        let token = headers
            .iter()
            .next()
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .filter(|s| !s.is_empty() && s.len() <= 8192 && !s.chars().any(char::is_whitespace))
            .ok_or_else(|| fail(401, "bearer_required"))?
            .to_owned();
        let operation = req.param::<String>("operation").unwrap_or_default();
        let input: Value = req
            .parse_json()
            .await
            .map_err(|_| fail(400, "invalid_json"))?;
        match operation.as_str() {
            "association-request" => app.request_association(&token, input).await,
            "session" => app.open(token, input).await,
            "call" => app.call(&token, input).await,
            "disconnect" => {
                empty_args(&input)?;
                app.disconnect(&token).await
            }
            _ => Err(fail(404, "not_found")),
        }
    }
    .await;
    match result {
        Ok(value) => res.render(Json(value)),
        Err(error) => {
            res.status_code(
                StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            );
            res.render(Json(json!({"code":error.code,"message":error.code})));
        }
    }
}
