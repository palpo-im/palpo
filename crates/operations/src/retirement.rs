//! Exact fleet-request retirement. This never asserts runtime termination or
//! settles usage; it proves deactivation, room removal and appservice denial.
use crate::{
    Result,
    api::App,
    digest, fail, now_ms, outbound,
    workflow::{Request, Workflows},
};
use palpo_hagency_contract::{MatrixUserId, RequestId};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    request_id: RequestId,
    agent_mxid: MatrixUserId,
}
impl App {
    pub fn with_retirement(mut self: Arc<Self>, token: String) -> Result<Arc<Self>> {
        if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_whitespace) {
            return Err(fail(400, "invalid_retirement_configuration"));
        }
        Arc::get_mut(&mut self)
            .ok_or_else(|| fail(409, "retirement_configuration_locked"))?
            .retirement_token = Some(token);
        Ok(self)
    }
    pub(crate) async fn retire_identity(
        &self,
        id: &str,
        bearer: &str,
        generation: Option<u64>,
        input: Value,
    ) -> Result<Value> {
        let _writer = self.mutation.lock().await;
        let intent: Intent = serde_json::from_value(input.clone())?;
        let fingerprint = digest(&input)?;
        let request = intent.request_id.as_str();
        let mxid = intent.agent_mxid.as_str();
        let state = self.store.lock().await.read()?;
        let fleet = outbound::authenticate_cleanup(&state, id, bearer, generation)?;
        let key = format!("{id}:{request}");
        let prior = &state["identityRetirements"][&key];
        if prior.is_object() && prior["digest"] != fingerprint {
            return Err(fail(409, "retirement_conflict"));
        }
        let suffix = format!(":{}", self.matrix.server().as_str());
        let local = mxid
            .strip_prefix(&format!("@{id}_"))
            .and_then(|s| s.strip_suffix(&suffix))
            .unwrap_or_default();
        if !(local.strip_prefix("en_").is_some_and(|v| {
            v.len() == 32
                && v.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) || local.strip_prefix("agent_").is_some_and(|v| {
            !v.is_empty()
                && v.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })) || fleet["representativeMxid"] == mxid
            || fleet["capabilities"]["approvalBotMxid"] == mxid
        {
            return Err(fail(403, "retirement_scope_mismatch"));
        }
        let workflows = Workflows::load(&state)?;
        // A failed provision may create its Matrix identity before it can
        // publish an observation. The immutable approved request still names
        // exactly one native identity; no caller chooses a different localpart.
        let expected_native = format!(
            "@{id}_en_{}:{}",
            &digest(&json!([id, request]))?[..32],
            self.matrix.server().as_str()
        );
        let action=workflows.actions.values().find(|a| matches!(&a.request,Request::Agent(r) if r.server_engagement_id.as_str()==id && r.id.as_str()==request));
        let known = action.is_some_and(|a| {
            a.state == "approved"
                && mxid == expected_native
                && workflows
                    .observations
                    .get(&a.id)
                    .is_none_or(|o| o["agentMxid"].is_null() || o["agentMxid"] == mxid)
        }) || state["requests"][&key]["fleetId"] == id
            && state["requests"][&key]["provider"]["agentMxid"] == mxid;
        if !known {
            return Err(fail(403, "retirement_scope_mismatch"));
        }
        if workflows.actions.values().any(|a| matches!(&a.request,Request::Agent(r) if r.server_engagement_id.as_str()==id && r.id.as_str()!=request)
            && workflows.observations.get(&a.id).is_some_and(|o|o["agentMxid"]==mxid && matches!(o["state"].as_str(),Some("pending"|"active"))))
            || state["requests"].as_object().into_iter().flat_map(|o|o.iter()).any(|(k,r)| k!=&key && r["fleetId"]==id && r["provider"]["agentMxid"]==mxid && matches!(r["state"].as_str(),Some("pending"|"active"))) {
            return Err(fail(409,"agent_still_allocated"));
        }
        // Reverify completed calls too: a cached receipt cannot hide a server
        // identity that was reactivated after retirement.
        let token = self
            .retirement_token
            .as_deref()
            .ok_or_else(|| fail(503, "retirement_unconfigured"))?;
        if !self.matrix.authenticate(token).await?.admin {
            return Err(fail(403, "admin_required"));
        }
        let user = self
            .matrix
            .segments(
                Method::GET,
                &["_palpo", "admin", "v2", "users", mxid],
                token,
                None,
                None,
            )
            .await?;
        if user["appservice_id"] != id || user["admin"] == true {
            return Err(fail(409, "identity_conflict"));
        }
        self.store.lock().await.transaction(|s| {
            if s["identityRetirements"].is_null(){s["identityRetirements"]=json!({});}
            if s["identityRetirements"][&key].is_null() {
                s["identityRetirements"][&key]=json!({"digest":fingerprint,"requestId":request,"fleetId":id,"mxid":mxid,"startedAt":now_ms(),"state":"pending"});
                s["audit"].as_array_mut().ok_or_else(||fail(503,"workflow_state_invalid"))?.push(json!({"atMs":now_ms(),"actor":format!("fleet:{id}"),"action":"agent.retire","fleetId":id,"target":mxid,"commandId":request,"result":"identity_cleanup_requested"}));
            }
            // Navigation checks this fence even before the next Hagency status.
            s["identityRetirements"][&key]["state"]=json!("pending");
            Ok(())
        })?;
        let result=async {
            if user["deactivated"]!=true {
                if !self.matrix.authenticate(token).await?.admin {return Err(fail(403,"admin_required"));}
                self.matrix.segments(Method::POST,&["_palpo","admin","v1","deactivate",mxid],token,None,Some(&json!({"erase":false}))).await?;
            }
            let user=self.matrix.segments(Method::GET,&["_palpo","admin","v2","users",mxid],token,None,None).await?;
            let rooms=self.matrix.segments(Method::GET,&["_palpo","admin","v1","users",mxid,"joined_rooms"],token,None,None).await?;
            if user["appservice_id"]!=id || user["admin"]==true || user["deactivated"]!=true || rooms["joined_rooms"].as_array().is_none_or(|r|!r.is_empty()) {return Err(fail(502,"retirement_unverified"));}
            let as_token=fleet["registration"]["as_token"].as_str().ok_or_else(||fail(503,"invalid_fleet"))?;
            match self.matrix.segments(Method::GET,&["_matrix","client","v3","account","whoami"],as_token,Some(mxid),None).await {
                Err(e) if matches!(e.status,401|403)=>{},
                Err(e)=>return Err(e),
                Ok(_)=>return Err(fail(502,"retirement_unverified")),
            }
            Ok(json!({"ok":true,"fleetId":id,"requestId":request,"agent":{"mxid":mxid,"state":"retired","matrixIdentity":"deactivated","appserviceAccess":"revoked","joinedRooms":[],"localTaskStop":"unconfirmed"}}))
        }.await;
        self.store.lock().await.transaction(|s| {
            let row = &mut s["identityRetirements"][&key];
            match &result {
                Ok(value) => {
                    row["state"] = json!("complete");
                    row["receipt"] = value.clone();
                    row["lastError"] = Value::Null;
                }
                Err(error) => {
                    row["state"] = json!("incomplete");
                    row["lastError"] = json!(error.code);
                }
            }
            row["updatedAt"] = json!(now_ms());
            Ok(())
        })?;
        result
    }
}
