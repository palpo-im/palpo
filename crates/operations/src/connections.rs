//! Owner-driven connection probes. Preparing/sending a probe never verifies it;
//! only the authenticated Matrix lane and Hagency receipt can do that.
use palpo_hagency_contract::{EngagementState, MatrixUserId};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::App;
use crate::outbound::{self, Limits};
use crate::workflow::Workflows;
use crate::{Result, fail, now_ms, secret};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    fleet_id: String,
}

pub(crate) fn view(
    state: &Value,
    w: &Workflows,
    id: &str,
    actor: &MatrixUserId,
    admin: bool,
    now: u64,
) -> Result<Value> {
    let e = w
        .authority
        .engagements
        .get(id)
        .ok_or_else(|| fail(404, "engagement_not_found"))?;
    let a = w.associations.values().find(|a| a.fleet_id == id);
    let designated = admin && a.is_some_and(|a| a.administrator_mxid == *actor);
    let owner = e.owner == *actor;
    let coordinator = e.coordinator == *actor && e.delegation_expires_at_ms > now;
    if !owner && !coordinator && !designated {
        return Err(fail(404, "engagement_not_found"));
    }
    let f = &state["fleets"][id];
    let active = !matches!(
        e.state,
        EngagementState::Suspended | EngagementState::Revoked
    ) && e.delegation_expires_at_ms > now
        && f["installation"] == "installed"
        && !matches!(f["state"].as_str(), Some("paused" | "revoked"));
    let verified = e.state == EngagementState::Verified
        && f["connection"]["generation"] == f["transport"]["generation"]
        && f["connection"]["verifiedAt"].is_string();
    let last_seen = f["transport"]["lastSeenAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .and_then(|d| u64::try_from(d.timestamp_millis()).ok());
    let online = active
        && last_seen
            .is_some_and(|at| at <= now.saturating_add(5000) && now.saturating_sub(at) < 90000);
    Ok(
        json!({"id":id,"name":f["name"].as_str().unwrap_or(id),"ownerMxid":e.owner,"coordinatorMxid":e.coordinator,
        "serverName":e.server,"state":e.state,"installation":f["installation"],"registrationGeneration":e.registration_generation,
        "delegationRevision":e.delegation_revision,"delegationExpiresAtMs":e.delegation_expires_at_ms,
        "connectionVerified":verified,"lastVerifiedAt":f["connection"]["verifiedAt"],"lastSeenAt":f["transport"]["lastSeenAt"],
        "connectivity":if online {"online"}else{"offline"},"canConnect":owner&&active,"canExport":active&&(designated||a.is_some_and(|a|a.intent.export_mxids.contains(actor))),
        "canInstall":designated&&a.is_some_and(|a|a.state=="approved"),"lastError":f["lastError"],"localTaskStop":f["localTaskStop"]}),
    )
}

impl App {
    pub(crate) async fn list_fleets(&self, bearer: &str, args: Value) -> Result<Value> {
        if args != json!({}) {
            return Err(fail(400, "invalid_arguments"));
        }
        let (_, _, identity) = self.authenticate(bearer).await?;
        let state = self.store.lock().await.read()?;
        let w = Workflows::load(&state)?;
        let rows = w
            .authority
            .engagements
            .keys()
            .filter_map(|id| view(&state, &w, id, &identity.user, identity.admin, now_ms()).ok())
            .collect::<Vec<_>>();
        Ok(json!({"fleets":rows}))
    }

    pub(crate) async fn connect_fleet(&self, bearer: &str, args: Value) -> Result<Value> {
        let input: Input = serde_json::from_value(args)?;
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        let state = self.store.lock().await.read()?;
        let w = Workflows::load(&state)?;
        let row = view(
            &state,
            &w,
            &input.fleet_id,
            &identity.user,
            identity.admin,
            now_ms(),
        )?;
        if row["canConnect"] != true {
            return Err(fail(403, "engagement_owner_required"));
        }
        let fleet = &state["fleets"][&input.fleet_id];
        let generation = fleet["transport"]["generation"]
            .as_u64()
            .ok_or_else(|| fail(503, "invalid_generation"))?;
        let key = format!("connection_{}_{}", input.fleet_id, generation);
        self.store.lock().await.transaction(|state|{
            if !state["preparations"].is_object() {state["preparations"]=json!({});}
            if state["preparations"].get(&key).is_none() {
                state["preparations"][&key]=json!({"fleetId":input.fleet_id,"projectId":format!("connection_{}",input.fleet_id),
                    "input":{"name":format!("{} · Connection",row["name"].as_str().unwrap_or("Hagency"))},"rooms":{}});
            }
            Ok(())
        })?;
        // The room is stable across credential rotations; its immutable binding
        // and current owner authority are checked on every recovery.
        let room = self
            .prepare_room(&key, "reception", &session.token, identity.user.as_str())
            .await?;
        let as_token = fleet["registration"]["as_token"]
            .as_str()
            .ok_or_else(|| fail(503, "registration_unavailable"))?;
        let representative = fleet["representativeMxid"]
            .as_str()
            .ok_or_else(|| fail(503, "registration_unavailable"))?;
        self.matrix
            .segments(
                Method::POST,
                &["_matrix", "client", "v3", "join", &room],
                as_token,
                Some(representative),
                Some(&json!({})),
            )
            .await?;
        let (_, _, current) = self.authenticate(bearer).await?;
        if current.user != identity.user {
            return Err(fail(403, "identity_changed"));
        }
        let probe=self.store.lock().await.transaction(|state|{
            let f=&mut state["fleets"][&input.fleet_id];
            if f["transport"]["generation"]!=generation {return Err(fail(409,"generation_conflict"));}
            if f["probe"]["generation"]!=generation {
                f["probe"]=json!({"generation":generation,"roomId":room,"challenge":secret(),"startedAtMs":now_ms()});
            }
            if f["probe"]["roomId"]!=room {return Err(fail(409,"reception_conflict"));}
            Ok(f["probe"].clone())
        })?;
        let challenge = probe["challenge"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_probe"))?;
        let event = if let Some(id) = probe["eventId"].as_str() {
            id.to_owned()
        } else {
            let event = self
                .matrix
                .segments(
                    Method::PUT,
                    &[
                        "_matrix",
                        "client",
                        "v3",
                        "rooms",
                        &room,
                        "send",
                        "com.hagency.connection.probe.v1",
                        &format!("probe_{challenge}"),
                    ],
                    as_token,
                    Some(representative),
                    Some(&json!({"v":1,"fleetId":input.fleet_id,"challenge":challenge})),
                )
                .await?;
            event["event_id"]
                .as_str()
                .filter(|id| {
                    id.starts_with('$') && id.len() <= 255 && !id.chars().any(char::is_control)
                })
                .ok_or_else(|| fail(502, "invalid_event_id"))?
                .to_owned()
        };
        self.store.lock().await.transaction_sql(|state,tx|{
            let f=&mut state["fleets"][&input.fleet_id];
            if f["transport"]["generation"]!=generation {return Err(fail(409,"generation_conflict"));}
            f["receptionRoomId"]=json!(room);f["probe"]["eventId"]=json!(event);
            outbound::enqueue(tx,f,"work","probe",&format!("probe_{challenge}"),&json!({"fleetId":input.fleet_id,"sourceRoomId":room,"sourceEventId":event,"challenge":challenge}),Limits::default())?;
            Ok(json!({"fleet":view(state,&Workflows::load(state)?,&input.fleet_id,&identity.user,identity.admin,now_ms())?}))
        })
    }
}
