//! Explicit room setup may join or repair a private room. Reads never do either.
use serde::Deserialize;

use super::*;

impl App {
    pub(crate) async fn actions_room(
        &self,
        bearer: &str,
        service: &str,
        args: Value,
    ) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Query {
            room_id: Option<String>,
        }
        let ensure = service == "palpo.actions.room.ensure";
        if ensure && args != json!({}) {
            return Err(fail(400, "invalid_arguments"));
        }
        let query: Query = serde_json::from_value(args)?;
        if query.room_id.as_ref().is_some_and(|s| {
            !s.starts_with('!') || s.len() > 255 || s.chars().any(char::is_control)
        }) {
            return Err(fail(400, "invalid_room_id"));
        }
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let actor = &identity.user;
        let Some(config) = &self.notifications else {
            return if ensure {
                Err(fail(501, "notifications_unavailable"))
            } else {
                Ok(json!({"room":null}))
            };
        };
        config.validate(self)?;
        if actor == &config.bot {
            return Err(fail(403, "action_room_owner_required"));
        }
        let bot = self
            .matrix
            .call(
                Method::GET,
                "/_matrix/client/v3/account/whoami",
                &config.token,
                None,
            )
            .await?;
        if bot["user_id"] != config.bot.as_str() || bot["is_guest"] == true {
            return Err(fail(403, "notification_identity_changed"));
        }
        if ensure {
            let id = match room(self, config, actor).await {
                Ok(id) => id,
                Err(error)
                    if error.code == "action_room_not_private"
                        || matches!(error.status, 403 | 404) =>
                {
                    self.store.lock().await.transaction(|state| {
                        let saved = &state["notificationRooms"][actor.as_str()];
                        let legacy = &state["actionInbox"]["rooms"][actor.as_str()];
                        if saved["roomId"].is_null() && legacy["roomId"].is_null() {
                            return Err(fail(409, "action_room_setup_unavailable"));
                        }
                        let revision = room_revision(state, actor)
                            .checked_add(1)
                            .filter(|n| *n < 9_007_199_254_740_991)
                            .ok_or_else(|| fail(409, "room_revision_exhausted"))?;
                        if !state["notificationRooms"].is_object() {
                            state["notificationRooms"] = json!({});
                        }
                        state["notificationRooms"][actor.as_str()] =
                            json!({"revision":revision,"botMxid":config.bot,"roomId":null});
                        let mut workflows = Workflows::load(state)?;
                        for notice in workflows.notices.values_mut().filter(|n| {
                            n["recipient"] == actor.as_str() && !n["delivery"].is_null()
                        }) {
                            notice["delivery"] = Value::Null;
                            notice["dueAt"] = json!(crate::now_ms());
                        }
                        workflows.save(state)?;
                        Ok(())
                    })?;
                    room(self, config, actor).await?
                }
                Err(error) => return Err(error),
            };
            // Recheck the user's session before the only user-authorized join.
            self.authenticate(bearer).await?;
            let events = self
                .matrix
                .room_state(&id, &config.token, config.bot.as_str())
                .await?;
            private_room(
                &events,
                &room_binding(self, config, actor, &self.store.lock().await.read()?),
            )?;
            if !events.as_array().is_some_and(|events| {
                events.iter().any(|e| {
                    e["type"] == "m.room.member"
                        && e["state_key"] == actor.as_str()
                        && e["content"]["membership"] == "join"
                })
            }) {
                self.matrix
                    .segments(
                        Method::POST,
                        &["_matrix", "client", "v3", "join", &id],
                        &session.token,
                        None,
                        Some(&json!({})),
                    )
                    .await?;
            }
            let result = verified(self, config, actor, Some(&id)).await?;
            self.authenticate(bearer).await?;
            if result.is_null() {
                return Err(fail(409, "action_room_not_private"));
            }
            Ok(result)
        } else {
            let result = verified(self, config, actor, query.room_id.as_deref()).await?;
            self.authenticate(bearer).await?;
            Ok(json!({"room":result}))
        }
    }
}
async fn verified(
    app: &App,
    config: &Configuration,
    actor: &MatrixUserId,
    wanted: Option<&str>,
) -> Result<Value> {
    let state = app.store.lock().await.read()?;
    let saved = &state["notificationRooms"][actor.as_str()];
    let selected = if saved.is_null() {
        &state["actionInbox"]["rooms"][actor.as_str()]
    } else {
        saved
    };
    if selected["botMxid"]
        .as_str()
        .is_some_and(|v| v != config.bot.as_str())
    {
        return Err(fail(409, "notification_identity_changed"));
    }
    let Some(id) = selected["roomId"]
        .as_str()
        .filter(|id| wanted.is_none_or(|w| *id == w))
    else {
        return Ok(Value::Null);
    };
    let events = app
        .matrix
        .room_state(id, &config.token, config.bot.as_str())
        .await?;
    private_room(&events, &room_binding(app, config, actor, &state))?;
    if !events.as_array().is_some_and(|events| {
        events.iter().any(|e| {
            e["type"] == "m.room.member"
                && e["state_key"] == actor.as_str()
                && e["content"]["membership"] == "join"
        })
    }) {
        return Err(fail(409, "action_room_join_required"));
    }
    Ok(
        json!({"v":1,"purpose":"my_actions","revision":room_revision(&state,actor),"account":actor,"roomId":id,"botMxid":config.bot,"serverName":app.matrix.server()}),
    )
}
