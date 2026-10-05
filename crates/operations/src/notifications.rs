//! Durable Matrix notices are a projection of Inbox. Delivery, dismissal and
//! room membership never decide an action or grant workflow authority.
use std::sync::Arc;

use palpo_hagency_contract::MatrixUserId;
use reqwest::{Method, Url};
use serde_json::{Value, json};

use crate::api::App;
use crate::workflow::Workflows;
use crate::{Result, digest, fail};

pub struct Configuration {
    pub bot: MatrixUserId,
    pub token: String,
    pub public_origin: String,
    /// UTC minutes after midnight; equal endpoints disable quiet hours.
    pub quiet_start: u16,
    pub quiet_end: u16,
}
impl Configuration {
    pub fn validate(&self, app: &App) -> Result<()> {
        let origin = Url::parse(&self.public_origin)
            .map_err(|_| fail(400, "invalid_notification_origin"))?;
        if !self.bot.belongs_to(app.matrix.server())
            || self.token.is_empty()
            || self.token.len() > 8192
            || self.quiet_start >= 1440
            || self.quiet_end >= 1440
            || !(origin.scheme() == "https"
                || origin.scheme() == "http"
                    && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(fail(400, "invalid_notification_configuration"));
        }
        Ok(())
    }
    fn quiet_until(&self, now: u64) -> Option<u64> {
        if self.quiet_start == self.quiet_end {
            return None;
        }
        let minute = (now / 60000) % 1440;
        let start = u64::from(self.quiet_start);
        let end = u64::from(self.quiet_end);
        let quiet = if start < end {
            minute >= start && minute < end
        } else {
            minute >= start || minute < end
        };
        quiet.then(|| {
            (now / 86400000) * 86400000 + (end + if minute >= end { 1440 } else { 0 }) * 60000
        })
    }
}

pub fn start(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Some(config) = &app.notifications {
                // Per-notice failures are persisted by tick; configuration or
                // authentication failures retry without printing credentials.
                let _ = tick(&app, config, crate::now_ms()).await;
            }
        }
    })
}

pub async fn tick(app: &App, config: &Configuration, now: u64) -> Result<()> {
    config.validate(app)?;
    let identity = app
        .matrix
        .call(
            Method::GET,
            "/_matrix/client/v3/account/whoami",
            &config.token,
            None,
        )
        .await?;
    if identity["user_id"] != config.bot.as_str() || identity["is_guest"] == true {
        return Err(fail(403, "notification_identity_changed"));
    }
    let due: Vec<String> = Workflows::load(&app.store.lock().await.read()?)?
        .notices
        .iter()
        .filter(|(_, n)| {
            n["cancelled"] != true
                && n["finished"] != true
                && n["dueAt"].as_u64().is_some_and(|t| t <= now)
        })
        .take(20)
        .map(|(id, _)| id.clone())
        .collect();
    for id in due {
        let _queue = app.mutation.lock().await;
        let w = Workflows::load(&app.store.lock().await.read()?)?;
        let Some(n) = w.notices.get(&id).filter(|n| {
            n["cancelled"] != true
                && n["finished"] != true
                && n["dueAt"].as_u64().is_some_and(|t| t <= now)
        }) else {
            continue;
        };
        let recipient: MatrixUserId = serde_json::from_value(n["recipient"].clone())?;
        let action_id = n["actionId"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_notice"))?;
        let view = match w.view(action_id, &recipient, now) {
            Ok(v) => Some(v),
            Err(e) if e.status == 404 => None,
            Err(e) => return Err(e),
        };
        let pending = view.as_ref().is_some_and(|v| v["needsMyAction"] == true);
        if recipient == config.bot
            || view.as_ref().is_none_or(|v| v["revision"] != n["revision"])
            || n["delivered"].as_u64().unwrap_or(0) > 0 && !pending
        {
            update(app, &id, |n| n["cancelled"] = json!(true)).await?;
            continue;
        }
        if let Some(until) = config.quiet_until(now) {
            update(app, &id, |n| n["dueAt"] = json!(until)).await?;
            continue;
        }
        let delivered = n["delivered"].as_u64().unwrap_or(0);
        let result=async {
            let room=room(app,config,&recipient).await?;
            let label=if pending {"A Palpo action needs your attention."}else{"A Palpo action has an update."};
            let route=format!("{}/_palpo/miniapp/action/{action_id}",config.public_origin.trim_end_matches('/'));
            let message=json!({"msgtype":"m.notice","body":format!("{label}\nOpen in Rinx: {route}"),
                "im.palpo.action.v1":{"v":1,"id":action_id,"revision":n["revision"],"appId":crate::api::APP_ID,"ownerMxid":recipient,"serverName":app.matrix.server()}});
            let sent=app.matrix.segments(Method::PUT,&["_matrix","client","v3","rooms",&room,"send","m.room.message",&format!("{id}_{delivered}")],&config.token,None,Some(&message)).await?;
            let event=sent["event_id"].as_str().filter(|id|id.starts_with('$') && id.len()<=255).ok_or_else(||fail(502,"invalid_notice_receipt"))?;
            Ok::<_,crate::Error>((room,event.to_owned()))
        }.await;
        match result {
            Ok((room, event)) => {
                update(app, &id, |n| {
                    let next = delivered + 1;
                    n["roomId"] = json!(room);
                    n["eventId"] = json!(event);
                    n["delivered"] = json!(next);
                    n["attempt"] = json!(0);
                    n["lastError"] = Value::Null;
                    n["finished"] = json!(!pending || next > 3);
                    let delay = [3600000u64, 86400000, 172800000]
                        .get(delivered as usize)
                        .copied()
                        .unwrap_or(0);
                    n["dueAt"] = json!(
                        n["createdAt"]
                            .as_u64()
                            .unwrap_or(now)
                            .saturating_add(delay)
                            .max(now.saturating_add(60000))
                    );
                })
                .await?;
                // The pinned board is independently retryable; losing its reply
                // must never repeat or advance a successfully delivered notice.
                if let Err(error) = board(app, config, &recipient, &room, now).await {
                    app.store.lock().await.transaction(|state| {
                        state["notificationRooms"][recipient.as_str()]["boardError"] =
                            json!(error.code);
                        Ok(())
                    })?;
                }
            }
            Err(error) => {
                update(app, &id, |n| {
                    let attempt = n["attempt"].as_u64().unwrap_or(0).saturating_add(1);
                    n["attempt"] = json!(attempt);
                    n["lastError"] = json!(error.code);
                    n["dueAt"] =
                        json!(now.saturating_add((1000u64 << attempt.min(12)).min(3600000)));
                })
                .await?
            }
        }
    }
    // Retry board projection failures even after every one-shot notice finished.
    let mut retry: Vec<(String, String)> = app.store.lock().await.read()?["notificationRooms"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(actor, r)| Some((actor.clone(), r["roomId"].as_str()?.to_owned())))
        .collect();
    if !retry.is_empty() {
        let offset = app.store.lock().await.transaction(|state| {
            let cursor =
                state["notificationBoardCursor"].as_u64().unwrap_or(0) as usize % retry.len();
            state["notificationBoardCursor"] = json!((cursor + 20) % retry.len());
            Ok(cursor)
        })?;
        retry.rotate_left(offset);
    }
    for (actor, id) in retry.into_iter().take(20) {
        if config.quiet_until(now).is_some() {
            break;
        }
        let _queue = app.mutation.lock().await;
        let actor: MatrixUserId = actor.try_into()?;
        if room(app, config, &actor).await.is_ok() {
            let _ = board(app, config, &actor, &id, now).await;
        }
    }
    Ok(())
}

async fn update(app: &App, id: &str, change: impl FnOnce(&mut Value)) -> Result<()> {
    app.store.lock().await.transaction(|state| {
        let mut w = Workflows::load(state)?;
        let n = w
            .notices
            .get_mut(id)
            .ok_or_else(|| fail(503, "notice_missing"))?;
        change(n);
        w.save(state)
    })
}

fn private_room(events: &Value, expected: &Value) -> Result<()> {
    let events = events
        .as_array()
        .filter(|s| s.len() <= 1000)
        .ok_or_else(|| fail(409, "action_room_not_private"))?;
    let content = |kind: &str, key: &str| {
        events
            .iter()
            .find(|e| e["type"] == kind && e["state_key"] == key)
            .map(|e| e["content"].clone())
            .unwrap_or(Value::Null)
    };
    let bot = expected["botMxid"].as_str().unwrap_or_default();
    let owner = expected["ownerMxid"].as_str().unwrap_or_default();
    let powers = content("m.room.power_levels", "");
    let creator = events
        .iter()
        .find(|e| e["type"] == "m.room.create" && e["state_key"] == "")
        .and_then(|e| {
            e["content"]["creator"]
                .as_str()
                .or_else(|| e["sender"].as_str())
        });
    if content("im.palpo.actions.v1", "") != *expected
        || creator != Some(bot)
        || content("m.room.join_rules", "")["join_rule"] != "invite"
        || content("m.room.history_visibility", "")["history_visibility"] != "invited"
        || !content("m.room.encryption", "").is_null()
        || content("m.room.create", "")["m.federate"] != false
        || powers["invite"] != 100
        || powers["state_default"] != 100
        || powers["users"][bot] != 100
        || powers["users_default"].as_i64().unwrap_or(0) != 0
        || powers["users"]
            .as_object()
            .is_none_or(|users| users.iter().any(|(u, p)| u != bot && p != 0))
        || [
            "m.room.power_levels",
            "m.room.join_rules",
            "m.room.history_visibility",
            "m.room.encryption",
            "im.palpo.actions.v1",
            "m.room.pinned_events",
        ]
        .iter()
        .any(|kind| {
            powers["events"]
                .get(*kind)
                .unwrap_or(&powers["state_default"])
                != 100
        })
        || content("m.room.member", bot)["membership"] != "join"
        || !matches!(
            content("m.room.member", owner)["membership"].as_str(),
            Some("join" | "invite")
        )
        || events.iter().any(|e| {
            e["type"] == "m.room.member"
                && matches!(e["content"]["membership"].as_str(), Some("join" | "invite"))
                && e["state_key"] != bot
                && e["state_key"] != owner
        })
    {
        return Err(fail(409, "action_room_not_private"));
    }
    Ok(())
}

async fn room(app: &App, config: &Configuration, actor: &MatrixUserId) -> Result<String> {
    let expected = json!({"v":1,"purpose":"my_actions","ownerMxid":actor,"botMxid":config.bot,"serverName":app.matrix.server()});
    let state = app.store.lock().await.read()?;
    let saved = &state["notificationRooms"][actor.as_str()];
    if saved["botMxid"]
        .as_str()
        .is_some_and(|bot| bot != config.bot.as_str())
    {
        return Err(fail(409, "notification_identity_changed"));
    }
    let legacy = &state["actionInbox"]["rooms"][actor.as_str()];
    if legacy["botMxid"]
        .as_str()
        .is_some_and(|bot| bot != config.bot.as_str())
    {
        return Err(fail(409, "notification_identity_changed"));
    }
    let mut id = saved["roomId"]
        .as_str()
        .or_else(|| legacy["roomId"].as_str())
        .map(str::to_owned);
    let alias = format!("palpo_actions_{}", &digest(&expected)?[..24]);
    if id.is_none() {
        let full_alias = format!("#{alias}:{}", app.matrix.server().as_str());
        match app
            .matrix
            .segments(
                Method::GET,
                &["_matrix", "client", "v3", "directory", "room", &full_alias],
                &config.token,
                None,
                None,
            )
            .await
        {
            Ok(found) => id = Some(matrix_room(&found)?),
            Err(e) if e.status == 404 => {}
            Err(e) => return Err(e),
        }
    }
    if id.is_none() && saved["attempted"] == true {
        let joined = app
            .matrix
            .call(
                Method::GET,
                "/_matrix/client/v3/joined_rooms",
                &config.token,
                None,
            )
            .await?;
        for candidate in joined["joined_rooms"]
            .as_array()
            .filter(|r| r.len() <= 1000)
            .ok_or_else(|| fail(409, "room_reconciliation_required"))?
        {
            let candidate = candidate
                .as_str()
                .ok_or_else(|| fail(502, "invalid_room_id"))?;
            let events = app
                .matrix
                .room_state(candidate, &config.token, config.bot.as_str())
                .await?;
            if events.as_array().is_some_and(|events| {
                events.iter().any(|e| {
                    e["type"] == "im.palpo.actions.v1"
                        && e["state_key"] == ""
                        && e["content"] == expected
                })
            }) {
                if id.is_some() {
                    return Err(fail(409, "ambiguous_room_binding"));
                }
                private_room(&events, &expected)?;
                id = Some(candidate.to_owned());
            }
        }
    }
    if id.is_none() {
        app.store.lock().await.transaction(|state| {
            if !state["notificationRooms"].is_object() {
                state["notificationRooms"] = json!({});
            }
            state["notificationRooms"][actor.as_str()] =
                json!({"botMxid":config.bot,"attempted":true});
            Ok(())
        })?;
        let created=app.matrix.call(Method::POST,"/_matrix/client/v3/createRoom",&config.token,Some(&json!({
            "name":"My Actions","room_alias_name":alias,"visibility":"private","preset":"private_chat","creation_content":{"m.federate":false},"invite":[actor],
            "power_level_content_override":{"users":{config.bot.as_str():100},"users_default":0,"invite":100,"state_default":100,"events_default":0},
            "initial_state":[{"type":"im.palpo.actions.v1","state_key":"","content":expected},
                {"type":"m.room.join_rules","state_key":"","content":{"join_rule":"invite"}},
                {"type":"m.room.history_visibility","state_key":"","content":{"history_visibility":"invited"}}]
        }))).await?;
        id = Some(matrix_room(&created)?);
    }
    let id = id.ok_or_else(|| fail(503, "room_preparation_failed"))?;
    private_room(
        &app.matrix
            .room_state(&id, &config.token, config.bot.as_str())
            .await?,
        &expected,
    )?;
    app.store.lock().await.transaction(|state| {
        if !state["notificationRooms"].is_object() {
            state["notificationRooms"] = json!({});
        }
        if !state["notificationRooms"][actor.as_str()].is_object() {
            state["notificationRooms"][actor.as_str()] = json!({});
        }
        state["notificationRooms"][actor.as_str()]["roomId"] = json!(id);
        state["notificationRooms"][actor.as_str()]["botMxid"] = json!(config.bot);
        Ok(())
    })?;
    Ok(id)
}
fn matrix_room(value: &Value) -> Result<String> {
    value["room_id"]
        .as_str()
        .filter(|r| r.starts_with('!') && r.len() <= 255 && !r.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| fail(502, "invalid_room_id"))
}
async fn board(
    app: &App,
    config: &Configuration,
    actor: &MatrixUserId,
    room: &str,
    now: u64,
) -> Result<()> {
    let state = app.store.lock().await.read()?;
    let w = Workflows::load(&state)?;
    let items: Vec<Value> = w
        .actions
        .keys()
        .filter_map(|id| w.view(id, actor, now).ok())
        .filter(|v| v["needsMyAction"] == true)
        .map(|v| json!({"id":v["id"],"revision":v["revision"]}))
        .collect();
    let fingerprint = digest(&json!({"actor":actor,"items":items}))?;
    if state["notificationRooms"][actor.as_str()]["boardDigest"] == fingerprint {
        return Ok(());
    }
    let body = json!({"msgtype":"m.notice","body":format!("My Actions: {} pending. Open Palpo Operations in Rinx to review your Inbox.",items.len()),
        "im.palpo.inbox.v1":{"v":1,"appId":crate::api::APP_ID,"ownerMxid":actor,"serverName":app.matrix.server(),"pending":items.len()}});
    let sent = app
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
                &format!("board_{fingerprint}"),
            ],
            &config.token,
            None,
            Some(&body),
        )
        .await?;
    let event = sent["event_id"]
        .as_str()
        .filter(|e| e.starts_with('$') && e.len() <= 255)
        .ok_or_else(|| fail(502, "invalid_notice_receipt"))?;
    app.matrix
        .segments(
            Method::PUT,
            &[
                "_matrix",
                "client",
                "v3",
                "rooms",
                room,
                "state",
                "m.room.pinned_events",
                "",
            ],
            &config.token,
            None,
            Some(&json!({"pinned":[event]})),
        )
        .await?;
    app.store.lock().await.transaction(|state| {
        state["notificationRooms"][actor.as_str()]["boardDigest"] = json!(fingerprint);
        state["notificationRooms"][actor.as_str()]["boardEventId"] = json!(event);
        state["notificationRooms"][actor.as_str()]["boardError"] = Value::Null;
        Ok(())
    })
}
