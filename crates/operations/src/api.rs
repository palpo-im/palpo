use std::collections::BTreeMap;
use std::sync::Arc;

use salvo::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::matrix::{Identity, Matrix};
use crate::store::Store;
use crate::{Result, digest, fail, now_ms, secret};

pub const APP_ID: &str = "im.palpo.operations";
pub const SERVICES: &[&str] = &[
    "palpo.intent.new",
    "palpo.session.open",
    "palpo.session.disconnect",
    "palpo.accounts.list",
    "palpo.accounts.open",
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
            accounts: None,
            account_limits: Mutex::new(crate::accounts::Limits::default()),
            ttl_ms,
        }))
    }

    async fn identity(&self, session: &Session, identity: &Identity) -> Result<Value> {
        Ok(
            json!({"version":1,"userId":identity.user,"isAdmin":identity.admin,
            "serverName":self.matrix.server(),"services":session.services,
            "features":{"accounts":self.accounts.is_some()}}),
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
        if input.bundle_digest.len() != 64
            || !input
                .bundle_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail(400, "invalid_bundle_digest"));
        }
        if input.app_id != APP_ID
            || input.services.is_empty()
            || input.services.len() > SERVICES.len()
            || input
                .services
                .iter()
                .any(|s| !SERVICES.contains(&s.as_str()))
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
        // Construct the response before publishing the session.
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
        match input.service.as_str() {
            "palpo.accounts.list" | "palpo.accounts.open" => {
                self.account_service(bearer, &input.service, input.args)
                    .await
            }
            "palpo.intent.new" => {
                empty_args(&input.args)?;
                Ok(json!({"requestId":&secret()[..40]}))
            }
            "palpo.session.open" => {
                empty_args(&input.args)?;
                self.identity(&session, &identity).await
            }
            "palpo.session.disconnect" => {
                empty_args(&input.args)?;
                self.disconnect(bearer).await
            }
            _ => Err(fail(403, "service_not_granted")),
        }
    }
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
