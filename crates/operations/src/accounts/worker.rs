use super::*;
use crate::{api::App, now_ms};
use reqwest::Method;
use std::sync::Arc;

pub fn start(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(2500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let _ = tick(&app).await;
        }
    })
}

async fn user(app: &App, c: &Configuration, id: &str) -> Result<Option<Value>> {
    match app
        .matrix
        .segments(
            Method::GET,
            &["_palpo", "admin", "v2", "users", id],
            &c.admin_token,
            None,
            None,
        )
        .await
    {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.status == 404 => Ok(None),
        Err(e) => Err(e),
    }
}
pub(super) async fn admin(app: &App, c: &Configuration, id: &str) -> Result<bool> {
    Ok(user(app, c, id).await?.is_some_and(|u| {
        u["admin"] == true
            && u["locked"] != true
            && u["deactivated"] != true
            && u["is_guest"] != true
            && u["appservice_id"].is_null()
    }))
}
pub(super) fn membership(events: &Value, who: &str, kinds: &[&str]) -> bool {
    events.as_array().is_some_and(|es| {
        es.iter().any(|e| {
            e["type"] == "m.room.member"
                && e["state_key"] == who
                && e["content"]["membership"]
                    .as_str()
                    .is_some_and(|m| kinds.contains(&m))
        })
    })
}
pub(super) async fn room(app: &App, c: &Configuration, id: &str) -> Result<Value> {
    let state = app
        .matrix
        .room_state(id, &c.bot_token, c.bot_mxid.as_str())
        .await?;
    let events = state
        .as_array()
        .ok_or_else(|| fail(502, "invalid_account_room"))?;
    let get = |kind: &str| {
        events
            .iter()
            .find(|e| e["type"] == kind && e["state_key"] == "")
            .map(|e| &e["content"])
            .unwrap_or(&Value::Null)
    };
    if get("m.room.join_rules")["join_rule"] != "invite"
        || !matches!(
            get("m.room.history_visibility")["history_visibility"].as_str(),
            Some("joined" | "invited")
        )
        || !get("m.room.encryption").is_null()
        || !membership(&state, c.bot_mxid.as_str(), &["join"])
        || events.iter().any(|e| {
            e["type"] == "m.room.member"
                && matches!(e["content"]["membership"].as_str(), Some("join" | "invite"))
                && e["state_key"] != c.bot_mxid.as_str()
                && !c.approvers.iter().any(|a| e["state_key"] == a.as_str())
        })
    {
        return Err(fail(409, "account_room_not_private"));
    }
    Ok(state)
}
fn room_id(state: &Value) -> Result<&str> {
    state["accountAccess"]["roomId"]
        .as_str()
        .filter(|s| s.starts_with('!') && s.len() <= 255 && !s.chars().any(char::is_control))
        .ok_or_else(|| fail(502, "invalid_account_room"))
}
async fn initialize(app: &App, c: &Configuration) -> Result<()> {
    let me = app
        .matrix
        .call(
            Method::GET,
            "/_matrix/client/v3/account/whoami",
            &c.bot_token,
            None,
        )
        .await?;
    if me["user_id"] != c.bot_mxid.as_str() || me["is_guest"] == true {
        return Err(fail(403, "account_bot_mismatch"));
    }
    if !app.matrix.authenticate(&c.admin_token).await?.admin {
        return Err(fail(403, "account_admin_unavailable"));
    }
    let mut allowed = Vec::new();
    for who in &c.approvers {
        if admin(app, c, who.as_str()).await? {
            allowed.push(who);
        }
    }
    if allowed.is_empty() {
        return Err(fail(403, "account_admin_unavailable"));
    }
    let state = app.store.lock().await.read()?;
    if state["accountAccess"]["roomId"].is_null() {
        let local = format!(
            "palpo_account_approvals_{}",
            &hash(c.bot_mxid.as_str())[..16]
        );
        let alias = format!("#{local}:{}", app.matrix.server().as_str());
        let result=match app.matrix.segments(Method::GET,&["_matrix","client","v3","directory","room",&alias],&c.bot_token,None,None).await {
            Ok(v)=>v,
            Err(e) if e.status==404=>app.matrix.call(Method::POST,"/_matrix/client/v3/createRoom",&c.bot_token,Some(&json!({
                "room_alias_name":local,"name":"Palpo · Account approvals","visibility":"private","preset":"private_chat","is_direct":false,
                "invite":allowed,"creation_content":{"m.federate":false},
                "power_level_content_override":{"users":{c.bot_mxid.as_str():100},"users_default":0,"invite":100,"events_default":0,"state_default":100},
                "initial_state":[{"type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"invited"}}]
            }))).await?,
            Err(e)=>return Err(e),
        };
        let id = result["room_id"]
            .as_str()
            .filter(|s| s.starts_with('!') && s.len() <= 255)
            .ok_or_else(|| fail(502, "invalid_account_room"))?;
        // Directory lookup reconciles a create whose response or local commit was lost.
        room(app, c, id).await?;
        app.store.lock().await.transaction(|s| {
            s["accountAccess"]["roomId"] = json!(id);
            Ok(())
        })?;
    }
    let state = app.store.lock().await.read()?;
    let id = room_id(&state)?;
    let events = room(app, c, id).await?;
    if state["accountAccess"]["historyUpgrade"].is_null()
        && events.as_array().unwrap().iter().any(|e| {
            e["type"] == "m.room.history_visibility"
                && e["content"]["history_visibility"] == "joined"
        })
    {
        app.store.lock().await.transaction(|s| {
            let ids: Vec<Value> = s["accountAccess"]["requests"]
                .as_object()
                .unwrap()
                .values()
                .filter(|r| r["status"] == "pending")
                .map(|r| r["id"].clone())
                .collect();
            s["accountAccess"]["historyUpgrade"] = json!({"pending":true,"requests":ids});
            Ok(())
        })?;
    }
    let current = app.store.lock().await.read()?;
    if current["accountAccess"]["historyUpgrade"]["pending"] == true {
        app.matrix
            .segments(
                Method::PUT,
                &[
                    "_matrix",
                    "client",
                    "v3",
                    "rooms",
                    id,
                    "state",
                    "m.room.history_visibility",
                    "",
                ],
                &c.bot_token,
                None,
                Some(&json!({"history_visibility":"invited"})),
            )
            .await?;
        let events = room(app, c, id).await?;
        if !events.as_array().unwrap().iter().any(|e| {
            e["type"] == "m.room.history_visibility"
                && e["content"]["history_visibility"] == "invited"
        }) {
            return Err(fail(409, "account_history_unconfirmed"));
        }
        app.store.lock().await.transaction(|s| {
            let ids = s["accountAccess"]["historyUpgrade"]["requests"]
                .as_array()
                .ok_or_else(|| fail(503, "invalid_account_state"))?
                .clone();
            for id in ids {
                let id = id
                    .as_str()
                    .ok_or_else(|| fail(503, "invalid_account_state"))?;
                let r = &mut s["accountAccess"]["requests"][id];
                if r["status"] != "pending" {
                    continue;
                }
                let old = r["sourceEventId"].clone();
                if !r["supersededSourceEventIds"].is_array() {
                    r["supersededSourceEventIds"] = json!([]);
                }
                r["supersededSourceEventIds"]
                    .as_array_mut()
                    .unwrap()
                    .push(old);
                r["notificationVersion"] =
                    json!(r["notificationVersion"].as_u64().unwrap_or(0) + 1);
                r["status"] = json!("notification_pending");
                r.as_object_mut().unwrap().remove("nextAttemptAt");
            }
            s["accountAccess"]["historyUpgrade"]["pending"] = json!(false);
            Ok(())
        })?;
    }
    app.store.lock().await.transaction(|s| {
        s["accountAccess"]["rustReady"] = json!(true);
        Ok(())
    })
}

fn card(c: &Configuration, row: &Value) -> Result<Value> {
    let text = |key: &str| row[key].as_str().unwrap_or_default();
    let expires = row["expiresAt"]
        .as_u64()
        .and_then(|v| i64::try_from(v).ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .ok_or_else(|| fail(503, "invalid_account_state"))?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(
        json!({"msgtype":"m.text","body":format!("Account request: {}\nName: {}\nReason: {}\nApprove creates an ordinary Matrix account. Project and agent resources require their separate approvals.",text("userId"),text("displayName"),text("reason")),
        "org.octos.approval_request":{"request_id":row["id"],"tool_name":"palpo.register_account","tool_args_digest":row["digest"],"title":format!("Register {}",text("userId")),"summary":format!("{}\n{}\nOrdinary user account; no administrator privileges.",text("displayName"),text("reason")),"risk_level":"normal","authorized_approvers":c.approvers,"expires_at":expires,"on_timeout":"notify"},
        "org.octos.actions":[{"id":"approve","label":"Approve","style":"primary"},{"id":"deny","label":"Reject","style":"danger"}]}),
    )
}
async fn send(
    app: &App,
    c: &Configuration,
    room: &str,
    id: &str,
    kind: &str,
    body: &Value,
) -> Result<String> {
    let transaction = format!("account_{id}_{kind}");
    let result = app
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
                "m.room.message",
                &transaction,
            ],
            &c.bot_token,
            None,
            Some(body),
        )
        .await?;
    result["event_id"]
        .as_str()
        .filter(|s| s.starts_with('$') && s.len() <= 1024)
        .map(str::to_owned)
        .ok_or_else(|| fail(502, "invalid_account_event"))
}
async fn notify(app: &App, c: &Configuration, id: &str, row: &Value, room_id: &str) -> Result<()> {
    if user(app, c, row["userId"].as_str().unwrap_or_default())
        .await?
        .is_some()
    {
        return app
            .store
            .lock()
            .await
            .transaction(|s| finish(s, id, "name_unavailable", now_ms()));
    }
    room(app, c, room_id).await?;
    let version = row["notificationVersion"].as_u64().unwrap_or(0);
    let kind = if version == 0 {
        "request".to_owned()
    } else {
        format!("request_v{version}")
    };
    let event = send(app, c, room_id, id, &kind, &card(c, row)?).await?;
    app.store.lock().await.transaction(|s| {
        s["accountAccess"]["requests"][id]["sourceEventId"] = json!(event);
        s["accountAccess"]["requests"][id]["status"] = json!("pending");
        Ok(())
    })
}
async fn decide(app: &App, c: &Configuration, room_id: &str, event: &Value) -> Result<()> {
    let response = &event["content"]["org.octos.approval_response"];
    if event["type"] != "m.room.message"
        || !response.is_object()
        || event["sender"] == c.bot_mxid.as_str()
    {
        return Ok(());
    }
    let Some(id) = response["request_id"].as_str().filter(|s| hex_id(s, 32)) else {
        return Ok(());
    };
    let state = app.store.lock().await.read()?;
    let row = &state["accountAccess"]["requests"][id];
    if row["status"] != "pending" {
        return Ok(());
    }
    let actor = event["sender"].as_str().unwrap_or_default();
    let valid = row["expiresAt"].as_u64().is_some_and(|n| now_ms() < n)
        && c.approvers.iter().any(|a| a.as_str() == actor)
        && matches!(response["decision"].as_str(), Some("approve" | "deny"))
        && response["source_event_id"] == row["sourceEventId"]
        && response["tool_args_digest"] == row["digest"]
        && event["content"]["m.relates_to"]["m.in_reply_to"]["event_id"] == row["sourceEventId"]
        && event["event_id"]
            .as_str()
            .is_some_and(|s| s.starts_with('$') && s.len() <= 1024);
    let authorized = if valid {
        membership(&room(app, c, room_id).await?, actor, &["join"]) && admin(app, c, actor).await?
    } else {
        false
    };
    app.store.lock().await.transaction(|s|{
        if !valid || !authorized {
            s["audit"].as_array_mut().ok_or_else(||fail(503,"invalid_account_state"))?.push(json!({"atMs":now_ms(),"actor":actor,"action":if valid{"account.unauthorized_decision"}else{"account.invalid_decision"},"target":id,"result":"refused"}));return Ok(());
        }
        let r=&mut s["accountAccess"]["requests"][id];
        if r["status"]!="pending" || r["sourceEventId"]!=row["sourceEventId"] {return Ok(());}
        if r["expiresAt"].as_u64().is_none_or(|n|now_ms()>=n){return finish(s,id,"expired",now_ms());}
        r["decidedBy"]=json!(actor);r["decisionEventId"]=event["event_id"].clone();r["decidedAt"]=json!(now_ms());
        if response["decision"]=="deny" {finish(s,id,"rejected",now_ms())} else {
            r["status"]=json!("approved");r.as_object_mut().unwrap().remove("nextAttemptAt");
            s["audit"].as_array_mut().unwrap().push(json!({"atMs":now_ms(),"actor":actor,"action":"account.approve","target":id,"result":"approved"}));Ok(())
        }
    })
}
async fn provision(
    app: &App,
    c: &Configuration,
    id: &str,
    row: &Value,
    room_id: &str,
) -> Result<()> {
    let mxid = row["userId"]
        .as_str()
        .ok_or_else(|| fail(503, "invalid_account_state"))?;
    let device = row["deviceId"]
        .as_str()
        .filter(|s| s.starts_with("PALPO_SIGNUP_") && s.len() == 61)
        .ok_or_else(|| fail(503, "invalid_account_state"))?;
    if let Some(existing) = user(app, c, mxid).await? {
        if row["attempted"] == true
            && existing["admin"] != true
            && existing["appservice_id"].is_null()
            && existing["deactivated"] != true
            && existing["is_guest"] != true
        {
            let proof = app
                .matrix
                .segments(
                    Method::GET,
                    &["_palpo", "admin", "v1", "whois", mxid],
                    &c.admin_token,
                    None,
                    None,
                )
                .await?;
            if proof["devices"]
                .as_object()
                .is_some_and(|m| m.contains_key(device))
            {
                return app
                    .store
                    .lock()
                    .await
                    .transaction(|s| finish(s, id, "registered", now_ms()));
            }
        }
        return app
            .store
            .lock()
            .await
            .transaction(|s| finish(s, id, "name_unavailable", now_ms()));
    }
    room(app, c, room_id).await?;
    if !admin(app, c, row["decidedBy"].as_str().unwrap_or_default()).await? {
        return Err(fail(403, "account_approver_revoked"));
    }
    let mut body = json!({"username":row["username"],"password":c.unseal(row)?,"device_id":device,"initial_device_display_name":"Palpo account registration"});
    app.store.lock().await.transaction(|s| {
        s["accountAccess"]["requests"][id]["attempted"] = json!(true);
        s["accountAccess"]["requests"][id]["status"] = json!("registering");
        Ok(())
    })?;
    let (mut status, mut result) = app.matrix.register(&body).await?;
    if status == 401 {
        let session = result["session"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
            .ok_or_else(|| fail(409, "unsupported_registration"))?;
        let kind = result["flows"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|f| {
                let stages = f["stages"].as_array()?;
                (stages.len() == 1)
                    .then(|| stages[0].as_str())
                    .flatten()
                    .filter(|s| matches!(*s, "m.login.registration_token" | "m.login.dummy"))
            })
            .ok_or_else(|| fail(409, "unsupported_registration"))?;
        body["auth"] = json!({"type":kind,"session":session});
        if kind == "m.login.registration_token" {
            body["auth"]["token"] = json!(c.registration_token);
        }
        room(app, c, room_id).await?;
        if !admin(app, c, row["decidedBy"].as_str().unwrap_or_default()).await? {
            return Err(fail(403, "account_approver_revoked"));
        }
        (status, result) = app.matrix.register(&body).await?;
    }
    if !(200..300).contains(&status) {
        return Err(fail(
            502,
            match result["errcode"].as_str() {
                Some("M_EXCLUSIVE") => "M_EXCLUSIVE",
                Some("M_INVALID_USERNAME") => "M_INVALID_USERNAME",
                Some("M_USER_IN_USE") => "M_USER_IN_USE",
                _ => "registration_failed",
            },
        ));
    }
    let token = result["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or_else(|| fail(502, "registration_unconfirmed"))?;
    if result["user_id"] != mxid || result["device_id"] != device {
        return Err(fail(502, "registration_unconfirmed"));
    }
    app.store
        .lock()
        .await
        .transaction(|s| finish(s, id, "registered", now_ms()))?;
    let _ = app
        .matrix
        .segments(
            Method::PUT,
            &["_matrix", "client", "v3", "profile", mxid, "displayname"],
            token,
            None,
            Some(&json!({"displayname":row["displayName"]})),
        )
        .await;
    let _ = app
        .matrix
        .call(
            Method::POST,
            "/_matrix/client/v3/logout",
            token,
            Some(&json!({})),
        )
        .await;
    Ok(())
}

/// One serialized pass. Every external effect has a stable identity and every
/// accepted decision is committed before registration. Secrets never enter logs.
pub async fn tick(app: &App) -> Result<()> {
    let Some(c) = &app.accounts else {
        return Ok(());
    };
    let _queue = app.mutation.lock().await;
    let result = run(app, c).await;
    if let Err(error) = &result {
        app.store.lock().await.transaction(|s| {
            s["accountAccess"]["rustReady"] = json!(false);
            s["accountAccess"]["lastError"] = json!(error.code);
            Ok(())
        })?;
    }
    result
}
async fn run(app: &App, c: &Configuration) -> Result<()> {
    // Live credentials and room privacy are revalidated on every pass.
    initialize(app, c).await?;
    app.store
        .lock()
        .await
        .transaction(|s| expire(s, now_ms()))?;
    let state = app.store.lock().await.read()?;
    let room_id = room_id(&state)?;
    room(app, c, room_id).await?;
    let ids: Vec<String> = state["accountAccess"]["requests"]
        .as_object()
        .ok_or_else(|| fail(503, "invalid_account_state"))?
        .iter()
        .filter(|(_, r)| {
            r["nextAttemptAt"].as_u64().is_none_or(|n| n <= now_ms())
                && (matches!(
                    r["status"].as_str(),
                    Some("notification_pending" | "approved" | "registering")
                ) || terminal(r)
                    && r["sourceEventId"].is_string()
                    && r["resultEventId"].is_null())
        })
        .take(25)
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        let result=async {
            let row=app.store.lock().await.read()?["accountAccess"]["requests"][&id].clone();
            if row["status"]=="notification_pending" {notify(app,c,&id,&row,room_id).await?;}
            if matches!(row["status"].as_str(),Some("approved"|"registering")){provision(app,c,&id,&row,room_id).await?;}
            let row=app.store.lock().await.read()?["accountAccess"]["requests"][&id].clone();
            if terminal(&row) && row["sourceEventId"].is_string() && row["resultEventId"].is_null() {
                room(app, c, room_id).await?;
                let event=send(app,c,room_id,&id,"result",&json!({"msgtype":"m.notice","body":format!("{}: {}.",row["userId"].as_str().unwrap_or_default(),row["status"].as_str().unwrap_or_default().replace('_'," ")),"m.relates_to":{"m.in_reply_to":{"event_id":row["sourceEventId"]}}})).await?;
                app.store.lock().await.transaction(|s|{s["accountAccess"]["requests"][&id]["resultEventId"]=json!(event);Ok(())})?;
            }
            Ok::<(),crate::Error>(())
        }.await;
        app.store.lock().await.transaction(|s| match &result {
            Err(e) if matches!(e.code, "M_EXCLUSIVE" | "M_INVALID_USERNAME") => {
                finish(s, &id, "name_unavailable", now_ms())
            }
            Err(e) => {
                let row = &mut s["accountAccess"]["requests"][&id];
                let n = row["retryCount"].as_u64().unwrap_or(0).saturating_add(1);
                row["lastError"] = json!(e.code);
                row["retryCount"] = json!(n);
                row["nextAttemptAt"] =
                    json!(now_ms() + 60000_u64.min(2500 * (1 << n.saturating_sub(1).min(5))));
                Ok(())
            }
            Ok(()) => {
                let row = s["accountAccess"]["requests"][&id].as_object_mut().unwrap();
                row.remove("nextAttemptAt");
                row.remove("retryCount");
                row.remove("lastError");
                Ok(())
            }
        })?;
    }
    let current = app.store.lock().await.read()?;
    let batch = app
        .matrix
        .forward_messages(
            room_id,
            &c.bot_token,
            current["accountAccess"]["cursor"].as_str(),
        )
        .await?;
    let events = batch["chunk"]
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or_else(|| fail(502, "invalid_account_history"))?;
    for event in events {
        decide(app, c, room_id, event).await?;
    }
    app.store.lock().await.transaction(|s| {
        if let Some(end) = batch["end"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096)
        {
            s["accountAccess"]["cursor"] = json!(end);
        }
        s["accountAccess"]["lastError"] = Value::Null;
        Ok(())
    })
}
