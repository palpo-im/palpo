//! Native destinations are resolved from current server state, never script URLs.
use palpo_hagency_contract::MatrixUserId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::App;
use crate::workflow::{Request, Workflows};
use crate::{Result, digest, fail, now_ms};

fn destination(state: &Value, id: &str, actor: &MatrixUserId, now: u64) -> Result<(Value, String)> {
    let mut w = Workflows::load(state)?;
    w.actions.retain(|_,a|matches!(&a.request,Request::Agent(r) if id==format!("{}:{}",r.server_engagement_id.as_str(),r.id.as_str())));
    let action = w
        .actions
        .values()
        .next()
        .ok_or_else(|| fail(404, "request_not_found"))?;
    let Request::Agent(request) = &action.request else {
        return Err(fail(404, "request_not_found"));
    };
    let view = crate::views::agents(
        state,
        &w,
        actor,
        now,
        serde_json::from_value(json!({"limit":1}))?,
    )?;
    if view["requests"][0]["usable"] != true {
        return Err(fail(409, "agent_not_ready"));
    }
    let observation = &w.observations[&action.id];
    let room = observation["targetRoomId"]
        .as_str()
        .filter(|id| id.starts_with('!') && id.len() <= 255 && !id.chars().any(char::is_control))
        .ok_or_else(|| fail(409, "agent_room_unavailable"))?;
    let agent = observation["agentMxid"]
        .as_str()
        .ok_or_else(|| fail(409, "agent_not_ready"))?;
    let fingerprint = digest(&json!({"action":action,"observation":observation,
        "authority":w.authority,"controls":w.agent_controls,"fleet":state["fleets"][request.server_engagement_id.as_str()]}))?;
    Ok((
        json!({"v":1,"requestId":id,"account":actor,"roomId":room,"agentMxid":agent}),
        fingerprint,
    ))
}
impl App {
    pub(crate) async fn open_agent_chat(&self, bearer: &str, input: Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Input {
            request_id: String,
        }
        let input: Input = serde_json::from_value(input)?;
        if input.request_id.len() > 170 {
            return Err(fail(400, "invalid_request_id"));
        }
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let prepared = destination(
            &self.store.lock().await.read()?,
            &input.request_id,
            &identity.user,
            now_ms(),
        )?;
        let room = prepared.0["roomId"].as_str().unwrap();
        let agent = prepared.0["agentMxid"].as_str().unwrap();
        let events = self
            .matrix
            .room_state(room, &session.token, identity.user.as_str())
            .await?;
        let events = events
            .as_array()
            .filter(|a| a.len() <= 10000)
            .ok_or_else(|| fail(502, "invalid_room_state"))?;
        for member in [identity.user.as_str(), agent] {
            if !events.iter().any(|e| {
                e["type"] == "m.room.member"
                    && e["state_key"] == member
                    && e["content"]["membership"] == "join"
            }) {
                return Err(fail(409, "agent_room_join_required"));
            }
        }
        self.authenticate(bearer).await?;
        if destination(
            &self.store.lock().await.read()?,
            &input.request_id,
            &identity.user,
            now_ms(),
        )? != prepared
        {
            return Err(fail(409, "agent_state_changed"));
        }
        Ok(prepared.0)
    }
}
