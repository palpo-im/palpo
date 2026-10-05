//! Recoverable owner-authorized Matrix setup. A persisted room plan precedes
//! createRoom; aliases and immutable creation bindings reconcile lost replies.
use reqwest::Method;
use serde_json::{Value, json};

use crate::api::App;
use crate::{Result, fail};

const BINDING: &str = "com.hagency.admin.binding.v1";

fn content<'a>(events: &'a [Value], kind: &str, key: &str) -> Option<&'a Value> {
    events
        .iter()
        .find(|e| e["type"] == kind && e["state_key"] == key)
        .map(|e| &e["content"])
}

pub(crate) fn validate(
    events: &Value,
    binding: &Value,
    owner: &str,
    encrypted: bool,
) -> Result<()> {
    let events = events
        .as_array()
        .filter(|e| e.len() <= 1000)
        .ok_or_else(|| fail(502, "invalid_room_state"))?;
    let fleet = binding["fleetId"]
        .as_str()
        .ok_or_else(|| fail(503, "invalid_room_plan"))?;
    let encryption = content(events, "m.room.encryption", "");
    let creator = events
        .iter()
        .find(|e| e["type"] == "m.room.create" && e["state_key"] == "")
        .map(|e| {
            e["content"]["creator"]
                .as_str()
                .or_else(|| e["sender"].as_str())
        });
    let levels = content(events, "m.room.power_levels", "");
    if content(events, BINDING, fleet) != Some(binding)
        || creator.flatten() != Some(owner)
        || content(events, "m.room.join_rules", "").is_none_or(|c| c["join_rule"] != "invite")
        || content(events, "m.room.member", owner).is_none_or(|c| c["membership"] != "join")
        || levels.is_none_or(|c| c["users"][owner].as_i64().unwrap_or(0) < 100)
        || if encrypted {
            encryption.is_none_or(|c| c["algorithm"] != "m.megolm.v1.aes-sha2")
        } else {
            encryption.is_some()
        }
    {
        return Err(fail(409, "room_authority_changed"));
    }
    if encrypted {
        let bot = format!(
            "@{fleet}_approval:{}",
            owner
                .split_once(':')
                .ok_or_else(|| fail(400, "invalid_owner"))?
                .1
        );
        if events.iter().any(|e| {
            e["type"] == "m.room.member"
                && matches!(e["content"]["membership"].as_str(), Some("join" | "invite"))
                && e["state_key"] != owner
                && e["state_key"] != bot
        }) {
            return Err(fail(409, "private_room_members_changed"));
        }
    }
    Ok(())
}

impl App {
    /// Called under the mutation queue, never while holding a SQLite transaction.
    pub(crate) async fn prepare_room(
        &self,
        key: &str,
        purpose: &str,
        token: &str,
        owner: &str,
    ) -> Result<String> {
        let plan = self.store.lock().await.read()?["preparations"][key].clone();
        let fleet = plan["fleetId"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_room_plan"))?;
        let project = plan["projectId"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_room_plan"))?;
        let encrypted = purpose == "approvals";
        let binding = json!({"v":1,"fleetId":fleet,"purpose":purpose,"projectId":project,"ownerMxid":owner,"authVersion":1});
        let alias_local = format!("hf_{project}_{purpose}");
        let alias = format!("#{alias_local}:{}", self.matrix.server().as_str());
        let mut room = plan["rooms"][purpose].as_str().map(str::to_owned);
        if room.is_none() {
            match self
                .matrix
                .segments(
                    Method::GET,
                    &["_matrix", "client", "v3", "directory", "room", &alias],
                    token,
                    None,
                    None,
                )
                .await
            {
                Ok(found) => room = Some(room_id(&found, self.matrix.server().as_str())?),
                Err(e) if e.status == 404 => {}
                Err(e) => return Err(e),
            }
        }
        // createRoom may have committed without storing its alias or returning a
        // response. Recover only a room bearing our complete immutable binding.
        if room.is_none() && plan["roomAttempts"][purpose] == true {
            let joined = self
                .matrix
                .call(Method::GET, "/_matrix/client/v3/joined_rooms", token, None)
                .await?;
            let joined = joined["joined_rooms"]
                .as_array()
                .filter(|r| r.len() <= 1000)
                .ok_or_else(|| fail(409, "room_reconciliation_required"))?;
            for candidate in joined {
                let candidate = candidate
                    .as_str()
                    .ok_or_else(|| fail(502, "invalid_room_state"))?;
                let events = self.matrix.room_state(candidate, token, owner).await?;
                if events.as_array().and_then(|e| content(e, BINDING, fleet)) == Some(&binding) {
                    if room.is_some() {
                        return Err(fail(409, "ambiguous_room_binding"));
                    }
                    validate(&events, &binding, owner, encrypted)?;
                    room = Some(candidate.to_owned());
                }
            }
        }
        if room.is_none() {
            self.store.lock().await.transaction(|state| {
                if !state["preparations"][key]["roomAttempts"].is_object() {
                    state["preparations"][key]["roomAttempts"] = json!({});
                }
                state["preparations"][key]["roomAttempts"][purpose] = json!(true);
                Ok(())
            })?;
            let invite = if encrypted {
                format!("@{fleet}_approval:{}", self.matrix.server().as_str())
            } else {
                format!("@{fleet}_representative:{}", self.matrix.server().as_str())
            };
            let mut initial = vec![
                json!({"type":BINDING,"state_key":fleet,"content":binding}),
                json!({"type":"m.room.join_rules","state_key":"","content":{"join_rule":"invite"}}),
            ];
            if encrypted {
                initial.push(json!({"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}));
            }
            let name = if encrypted {
                format!(
                    "{} · Approvals",
                    plan["input"]["name"].as_str().unwrap_or("Project")
                )
            } else {
                plan["input"]["name"].as_str().unwrap_or("Project").into()
            };
            let body = json!({"visibility":"private","preset":"private_chat","room_version":"11","room_alias_name":alias_local,
                "name":name,"invite":[invite],"initial_state":initial,
                "power_level_content_override":{"users":{owner:100,invite:50},"users_default":0,"invite":50}});
            let result = self
                .matrix
                .call(
                    Method::POST,
                    "/_matrix/client/v3/createRoom",
                    token,
                    Some(&body),
                )
                .await?;
            room = Some(room_id(&result, self.matrix.server().as_str())?);
        }
        let room = room.ok_or_else(|| fail(503, "room_preparation_failed"))?;
        validate(
            &self.matrix.room_state(&room, token, owner).await?,
            &binding,
            owner,
            encrypted,
        )?;
        self.store.lock().await.transaction(|state| {
            if !state["preparations"][key]["rooms"].is_object() {
                state["preparations"][key]["rooms"] = json!({});
            }
            state["preparations"][key]["rooms"][purpose] = json!(room);
            Ok(())
        })?;
        Ok(room)
    }
}

fn room_id(value: &Value, server: &str) -> Result<String> {
    value["room_id"]
        .as_str()
        .filter(|r| {
            r.starts_with('!')
                && r.ends_with(&format!(":{server}"))
                && r.len() <= 255
                && !r.chars().any(|c| c.is_whitespace() || c.is_control())
        })
        .map(str::to_owned)
        .ok_or_else(|| fail(502, "invalid_room_id"))
}
