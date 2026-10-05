use std::collections::BTreeMap;
use std::sync::Arc;

use salvo::prelude::*;

use super::*;
use crate::api::App;

#[derive(Default)]
pub(crate) struct Limits(BTreeMap<(String, bool), (u64, u32)>);
impl Limits {
    fn check(&mut self, peer: String, status: bool, now: u64) -> Result<()> {
        self.0.retain(|(_, status), (at, _)| {
            now.saturating_sub(*at) < if *status { 60000 } else { 3600000 }
        });
        let key = (peer, status);
        if !self.0.contains_key(&key) && self.0.len() >= 10000 {
            return Err(fail(429, "account_rate_limited"));
        }
        let row = self.0.entry(key).or_insert((now, 0));
        if row.1 >= if status { 180 } else { 30 } {
            return Err(fail(429, "account_rate_limited"));
        }
        row.1 += 1;
        Ok(())
    }
}

fn public(app: &App, state: &Value) -> Value {
    json!({"enabled":app.accounts.is_some(),"ready":app.accounts.is_some() && state["accountAccess"]["rustReady"]==true,"serverName":app.matrix.server()})
}

pub(crate) fn router() -> Router {
    Router::with_path("api")
        .hoop(salvo::size_limiter::max_size(16384))
        .push(Router::with_path("account-access").get(account_access))
        .push(
            Router::with_path("account-requests")
                .post(account_request)
                .push(Router::with_path("status").post(account_status)),
        )
}

fn guard(req: &Request, app: &App, post: bool) -> Result<()> {
    if req.headers().get("host").and_then(|h| h.to_str().ok()) != Some(app.host.as_str()) {
        return Err(fail(403, "host_forbidden"));
    }
    if req
        .headers()
        .get("origin")
        .is_some_and(|h| h.to_str().ok() != Some(app.public_origin.as_str()))
        || req
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|h| !matches!(h.to_str().ok(), Some("same-origin" | "none")))
    {
        return Err(fail(403, "origin_forbidden"));
    }
    if post
        && req
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|h| h.split(';').next().unwrap_or_default())
            != Some("application/json")
    {
        return Err(fail(415, "json_required"));
    }
    Ok(())
}
fn render(res: &mut Response, result: Result<Value>, accepted: bool) {
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    match result {
        Ok(value) => {
            if accepted {
                res.status_code(StatusCode::ACCEPTED);
            }
            res.render(Json(value));
        }
        Err(error) => {
            res.status_code(
                StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            );
            res.render(Json(json!({"code":error.code,"message":error.code})));
        }
    }
}
#[handler]
async fn account_access(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let result = async {
        let app = depot
            .get_typed::<Arc<App>>()
            .map_err(|_| fail(503, "service_unavailable"))?;
        guard(req, app, false)?;
        Ok(public(app, &app.store.lock().await.read()?))
    }
    .await;
    render(res, result, false);
}
#[handler]
async fn account_request(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    render(res, mutate(req, depot, false).await, true);
}
#[handler]
async fn account_status(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    render(res, mutate(req, depot, true).await, false);
}
async fn mutate(req: &mut Request, depot: &Depot, status: bool) -> Result<Value> {
    let app = depot
        .get_typed::<Arc<App>>()
        .map_err(|_| fail(503, "service_unavailable"))?;
    guard(req, app, true)?;
    // Rate keys use the socket peer, never applicant-controlled proxy headers.
    let peer = match req.remote_addr() {
        salvo::conn::SocketAddr::IPv4(a) => a.ip().to_string(),
        salvo::conn::SocketAddr::IPv6(a) => a.ip().to_string(),
        _ => "local".to_owned(),
    };
    app.account_limits
        .lock()
        .await
        .check(peer, status, crate::now_ms())?;
    let input: Value = req
        .parse_json()
        .await
        .map_err(|_| fail(400, "invalid_json"))?;
    let _queue = app.mutation.lock().await;
    let config = app
        .accounts
        .as_ref()
        .ok_or_else(|| fail(503, "account_requests_unavailable"))?;
    app.store.lock().await.transaction(|state| {
        let value = if status {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Query {
                id: String,
                receipt: String,
            }
            let query: Query = serde_json::from_value(input)?;
            receipt(state, &query.id, &query.receipt)?;
            expire(state, crate::now_ms())?;
            view(receipt(state, &query.id, &query.receipt)?)
        } else {
            submit(state, config, input, app.matrix.server(), crate::now_ms())?
        };
        Ok(json!({"request":value}))
    })
}

impl App {
    pub(crate) async fn account_service(
        &self,
        bearer: &str,
        service: &str,
        args: Value,
    ) -> Result<Value> {
        let _queue = self.mutation.lock().await;
        let (_, _, identity) = self.authenticate(bearer).await?;
        if !identity.admin {
            return Err(fail(403, "admin_required"));
        }
        let state = self.store.lock().await.read()?;
        if service == "palpo.accounts.list" {
            if args != json!({}) {
                return Err(fail(400, "invalid_arguments"));
            }
            let mut result = public(self, &state);
            result["roomId"] = state["accountAccess"]["roomId"].clone();
            result["botMxid"] = self
                .accounts
                .as_ref()
                .map(|c| json!(c.bot_mxid))
                .unwrap_or(Value::Null);
            result["lastError"] = state["accountAccess"]["lastError"].clone();
            let rows: Vec<Value> = state["accountAccess"]["requests"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, r)| {
                    let mut v = view(r);
                    for key in ["displayName", "reason", "decidedBy", "lastError"] {
                        v[key] = r[key].clone();
                    }
                    v["canOpen"] = json!(
                        self.accounts
                            .as_ref()
                            .is_some_and(|c| c.approvers.contains(&identity.user))
                            && r["sourceEventId"].is_string()
                            && state["accountAccess"]["roomId"].is_string()
                    );
                    v
                })
                .collect();
            result["requests"] = json!(rows);
            return Ok(result);
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Query {
            request_id: String,
        }
        let query: Query = serde_json::from_value(args)?;
        if !hex_id(&query.request_id, 32) {
            return Err(fail(400, "invalid_request"));
        }
        let config = self
            .accounts
            .as_ref()
            .ok_or_else(|| fail(503, "account_requests_unavailable"))?;
        if !config.approvers.contains(&identity.user)
            || !worker::admin(self, config, identity.user.as_str()).await?
        {
            return Err(fail(403, "account_approver_required"));
        }
        let row = &state["accountAccess"]["requests"][&query.request_id];
        let room = state["accountAccess"]["roomId"]
            .as_str()
            .ok_or_else(|| fail(409, "account_notification_pending"))?;
        let event = row["sourceEventId"]
            .as_str()
            .ok_or_else(|| fail(409, "account_notification_pending"))?;
        let events = worker::room(self, config, room).await?;
        if !worker::membership(&events, identity.user.as_str(), &["join", "invite"]) {
            return Err(fail(403, "account_room_membership_required"));
        }
        let source = self
            .matrix
            .segments(
                reqwest::Method::GET,
                &["_matrix", "client", "v3", "rooms", room, "event", event],
                &config.bot_token,
                None,
                None,
            )
            .await?;
        let request = &source["content"]["org.octos.approval_request"];
        if source["event_id"] != event
            || source["type"] != "m.room.message"
            || source["sender"] != config.bot_mxid.as_str()
            || request["request_id"] != query.request_id
            || request["tool_name"] != "palpo.register_account"
            || request["tool_args_digest"] != row["digest"]
            || !request["authorized_approvers"]
                .as_array()
                .is_some_and(|a| a.contains(&json!(identity.user)))
        {
            return Err(fail(409, "account_source_changed"));
        }
        self.authenticate(bearer).await?;
        if !worker::admin(self, config, identity.user.as_str()).await? {
            return Err(fail(403, "account_approver_required"));
        }
        Ok(
            json!({"v":1,"requestId":query.request_id,"account":identity.user,"roomId":room,"eventId":event}),
        )
    }
}
