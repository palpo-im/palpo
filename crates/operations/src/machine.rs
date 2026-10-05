use std::sync::Arc;
use std::time::Duration;

use salvo::http::Method;
use salvo::prelude::*;
use serde_json::{Value, json};

use crate::api::App;
use crate::outbound::{self, Limits};
use crate::{Result, fail, now_ms};

pub fn router() -> Router {
    Router::new()
        .push(
            Router::with_path("api/fleet/v2/{fleet}/{operation}")
                .hoop(salvo::size_limiter::max_size(1024 * 1024))
                .get(machine)
                .post(machine),
        )
        .push(
            Router::with_path("api/relay/v2/{fleet}/transactions/{transaction}")
                .hoop(salvo::size_limiter::max_size(1024 * 1024))
                .put(relay),
        )
        .push(
            Router::with_path("api/relay/v2/{fleet}/_matrix/app/v1/transactions/{transaction}")
                .hoop(salvo::size_limiter::max_size(1024 * 1024))
                .put(relay),
        )
        .push(Router::with_path("api/relay/v2/{fleet}/{kind}/{identity}").get(query_identity))
        .push(
            Router::with_path("api/relay/v2/{fleet}/_matrix/app/v1/{kind}/{identity}")
                .get(query_identity),
        )
}

fn headers(req: &mut Request, app: &App, is_relay: bool) -> Result<(String, String, Option<u64>)> {
    let expected = if is_relay {
        &app.relay_host
    } else {
        &app.transport_host
    };
    let expected = expected
        .as_ref()
        .ok_or_else(|| fail(501, "outbound_unconfigured"))?;
    if req.headers().contains_key("origin")
        || req.headers().contains_key("cookie")
        || req.headers().get("host").and_then(|s| s.to_str().ok()) != Some(expected.as_str())
    {
        return Err(fail(403, "host_forbidden"));
    }
    let fleet = req.param::<String>("fleet").unwrap_or_default();
    if fleet.len() != 35
        || !fleet.starts_with("hf_")
        || !fleet[3..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(fail(404, "fleet_not_found"));
    }
    let auth = req.headers().get_all("authorization");
    if auth.iter().count() > 1 {
        return Err(fail(401, "transport_unauthorized"));
    }
    let bearer = auth
        .iter()
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned);
    let query = if is_relay {
        req.query::<String>("access_token")
    } else {
        None
    };
    if bearer.is_some() && query.is_some() {
        return Err(fail(401, "ambiguous_credential"));
    }
    let token = bearer
        .or(query)
        .filter(|s| !s.is_empty() && s.len() <= 8192 && !s.chars().any(char::is_whitespace))
        .ok_or_else(|| fail(401, "transport_unauthorized"))?;
    let generation = req
        .headers()
        .get("x-hagency-generation")
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok().filter(|n| n.to_string() == s));
    if req.method() != Method::GET
        && req
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or_default())
            != Some("application/json")
    {
        return Err(fail(415, "json_required"));
    }
    Ok((fleet, token, generation))
}

fn render(res: &mut Response, result: Result<Value>) {
    res.headers_mut()
        .insert("Cache-Control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("X-Content-Type-Options", "nosniff".parse().unwrap());
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

#[handler]
async fn machine(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let result = async {
        let app = depot
            .get_typed::<Arc<App>>()
            .map_err(|_| fail(503, "service_unavailable"))?;
        let (id, token, generation) = headers(req, app, false)?;
        let operation = req.param::<String>("operation").unwrap_or_default();
        if operation == "retire-agent" && req.method() == Method::POST {
            let input = req
                .parse_json::<Value>()
                .await
                .map_err(|_| fail(400, "invalid_retirement"))?;
            return app.retire_identity(&id, &token, generation, input).await;
        }
        if operation == "poll" && req.method() == Method::GET {
            let lane = req.query::<String>("lane").unwrap_or_default();
            let consumer = req.query::<String>("consumer").unwrap_or_default();
            let wait = req
                .query::<String>("wait")
                .unwrap_or_else(|| "25000".into());
            let wait = wait
                .parse::<u64>()
                .ok()
                .filter(|n| *n <= 25000)
                .ok_or_else(|| fail(400, "invalid_poll"))?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
            loop {
                let response = app.store.lock().await.transaction_sql(|state, tx| {
                    let fleet = outbound::authenticate(state, &id, &token, generation, false)?;
                    outbound::claim(tx, &fleet, &lane, &consumer, now_ms(), Limits::default())
                })?;
                if !response["delivery"].is_null() || tokio::time::Instant::now() >= deadline {
                    return Ok(response);
                }
                tokio::time::sleep_until(
                    (tokio::time::Instant::now() + Duration::from_millis(100)).min(deadline),
                )
                .await;
            }
        }
        if operation == "ack" && req.method() == Method::POST {
            let input: outbound::Ack = req
                .parse_json()
                .await
                .map_err(|_| fail(400, "invalid_ack"))?;
            return app.store.lock().await.transaction_sql(|state, tx| {
                let fleet = outbound::authenticate(state, &id, &token, generation, false)?;
                outbound::ack(tx, &fleet, &input, now_ms())
            });
        }
        if operation == "updates" && req.method() == Method::POST {
            let input: Value = req
                .parse_json()
                .await
                .map_err(|_| fail(400, "invalid_update"))?;
            let _writer = app.mutation.lock().await;
            let before = app.store.lock().await.transaction(|state| {
                outbound::authenticate(state, &id, &token, generation, false)
            })?;
            if !crate::updates::sequence(&before, &input)? {
                return Ok(json!({"ok":true}));
            }
            let room = if input["probeReceipts"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
            {
                let field = |path: &str| {
                    before
                        .pointer(path)
                        .and_then(Value::as_str)
                        .ok_or_else(|| fail(409, "probe_binding_conflict"))
                };
                Some(
                    app.matrix
                        .room_state(
                            field("/probe/roomId")?,
                            field("/registration/as_token")?,
                            field("/representativeMxid")?,
                        )
                        .await?,
                )
            } else {
                None
            };
            return app.store.lock().await.transaction_sql(|state, tx| {
                let fleet = outbound::authenticate(state, &id, &token, generation, false)?;
                if fleet != before {
                    return Err(fail(409, "fleet_binding_changed"));
                }
                crate::updates::apply(
                    state,
                    tx,
                    fleet,
                    &input,
                    room.as_ref(),
                    app.matrix.server(),
                    now_ms(),
                )
            });
        }
        Err(fail(501, "machine_operation_not_implemented"))
    }
    .await;
    render(res, result);
}

#[handler]
async fn relay(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let result = async {
        let app = depot
            .get_typed::<Arc<App>>()
            .map_err(|_| fail(503, "service_unavailable"))?;
        let (id, token, generation) = headers(req, app, true)?;
        let transaction = req
            .param::<String>("transaction")
            .ok_or_else(|| fail(400, "invalid_transaction"))?;
        let body: Value = req
            .parse_json()
            .await
            .map_err(|_| fail(400, "invalid_transaction"))?;
        app.store.lock().await.transaction_sql(|state, tx| {
            let mut fleet = outbound::authenticate(state, &id, &token, generation, true)?;
            outbound::relay(tx, &mut fleet, &transaction, body, Limits::default())?;
            state["fleets"][&id] = fleet;
            Ok(json!({}))
        })
    }
    .await;
    render(res, result);
}

#[handler]
async fn query_identity(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let result = async {
        let app = depot
            .get_typed::<Arc<App>>()
            .map_err(|_| fail(503, "service_unavailable"))?;
        let (id, token, generation) = headers(req, app, true)?;
        let fleet = outbound::authenticate(
            &app.store.lock().await.read()?,
            &id,
            &token,
            generation,
            true,
        )?;
        let entity = req.param::<String>("identity").unwrap_or_default();
        let kind = req.param::<String>("kind").unwrap_or_default();
        let known = kind == "users"
            && entity.starts_with(&format!("@{id}_"))
            && entity
                .split_once(':')
                .is_some_and(|(_, server)| server == app.matrix.server().as_str())
            && (fleet["representativeMxid"] == entity
                || fleet["agents"].as_object().is_some_and(|a| {
                    a.values()
                        .any(|v| v["mxid"] == entity && v["state"] == "registered")
                }));
        if !known {
            return Err(fail(404, "identity_not_found"));
        }
        Ok(json!({}))
    }
    .await;
    render(res, result);
}
