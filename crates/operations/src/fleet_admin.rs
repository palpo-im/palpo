//! Matrix administrators manage registration credentials. This never grants
//! coordinator authority or claims that an offline runtime has stopped.
use crate::{
    Result, api::App, connections, digest, engagement_setup::registration_matches, fail, now_ms,
    outbound, secret, workflow::Workflows,
};
use palpo_hagency_contract::{MatrixUserId, RequestId};
use reqwest::Method;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) fn ensure_active(state: &Value, id: &str) -> Result<()> {
    let f = &state["fleets"][id];
    if !f.is_null()
        && (f["pendingAdminOperation"].is_string()
            || matches!(
                f["state"].as_str(),
                Some("paused" | "revoked" | "resuming" | "rotating")
            ))
    {
        return Err(fail(409, "fleet_inactive"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateIntent {
    fleet_id: String,
    request_id: RequestId,
    action: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rotation {
    fleet_id: String,
    request_id: RequestId,
    #[serde(default)]
    rotate: bool,
}
fn fleet<'a>(state: &'a Value, id: &str) -> Result<&'a Value> {
    state["fleets"]
        .get(id)
        .filter(|f| f["id"] == id && f["registration"].is_object())
        .ok_or_else(|| fail(404, "fleet_not_found"))
}
fn audit(
    state: &mut Value,
    actor: &MatrixUserId,
    action: &str,
    id: &str,
    operation: &str,
    result: &str,
) -> Result<()> {
    state["audit"].as_array_mut().ok_or_else(||fail(503,"workflow_state_invalid"))?.push(json!({"atMs":now_ms(),"actor":actor,"action":action,"fleetId":id,"commandId":operation,"result":result}));
    Ok(())
}
fn operations(state: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
    if state["fleetAdminOperations"].is_null() {
        state["fleetAdminOperations"] = json!({});
    }
    state["fleetAdminOperations"]
        .as_object_mut()
        .ok_or_else(|| fail(503, "workflow_state_invalid"))
}
fn checked_operation(state: &Value, request: &str, fingerprint: &str) -> Result<Option<Value>> {
    let row = &state["fleetAdminOperations"][request];
    if row.is_null() {
        return Ok(None);
    }
    if row["digest"] != fingerprint {
        return Err(fail(409, "idempotency_conflict"));
    }
    Ok(Some(row.clone()))
}
fn no_pending(f: &Value) -> Result<()> {
    if f["pendingAdminOperation"].is_string() {
        return Err(fail(409, "fleet_operation_pending"));
    }
    Ok(())
}

fn association_status(state: &mut Value, id: &str, execution: &str) -> Result<()> {
    let mut workflows = Workflows::load(state)?;
    let now = now_ms();
    let mut changed = Vec::new();
    for association in workflows.associations.values_mut() {
        if association.fleet_id == id
            && association.state == "approved"
            && association.execution != execution
        {
            association.execution = execution.into();
            association.revision = association
                .revision
                .checked_add(1)
                .ok_or_else(|| fail(409, "revision_exhausted"))?;
            association.updated_at = now;
            association.last_error = None;
            changed.push(association.id.clone());
        }
    }
    for id in changed {
        workflows.notify_association(&id, now)?;
    }
    workflows.save(state)
}

impl App {
    async fn installed_registration(&self, f: &Value, token: &str) -> Result<Value> {
        let id = f["id"].as_str().ok_or_else(|| fail(503, "invalid_fleet"))?;
        let value = self
            .matrix
            .segments(
                Method::GET,
                &["_palpo", "admin", "v1", "appservices", id],
                token,
                None,
                None,
            )
            .await?;
        if !registration_matches(&value, &f["registration"]) {
            return Err(fail(409, "registration_drift"));
        }
        Ok(value)
    }
    pub(crate) async fn fleet_admin(
        &self,
        bearer: &str,
        service: &str,
        args: Value,
    ) -> Result<Value> {
        let _queue = self.mutation.lock().await;
        let (_, session, identity) = self.authenticate(bearer).await?;
        if !identity.admin {
            return Err(fail(403, "admin_required"));
        }
        match service {
            "palpo.activity.list" => {
                if args != json!({}) {
                    return Err(fail(400, "invalid_arguments"));
                }
                let state = self.store.lock().await.read()?;
                let events: Vec<Value> = state["audit"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .rev()
                    .take(200)
                    .map(|r| {
                        let mut v = json!({});
                        for k in [
                            "actor",
                            "action",
                            "result",
                            "fleetId",
                            "target",
                            "commandId",
                            "actionId",
                            "decision",
                            "atMs",
                        ] {
                            if let Some(value) = r.get(k) {
                                v[k] = value.clone();
                            }
                        }
                        v["at"] = r["at"]
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| {
                                r["atMs"]
                                    .as_u64()
                                    .and_then(|n| i64::try_from(n).ok())
                                    .and_then(chrono::DateTime::from_timestamp_millis)
                                    .map(|t| t.to_rfc3339())
                            })
                            .map(Value::String)
                            .unwrap_or(Value::Null);
                        v
                    })
                    .collect();
                Ok(json!({"events":events}))
            }
            "palpo.fleets.queue" => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Query {
                    fleet_id: String,
                }
                let q: Query = serde_json::from_value(args)?;
                self.store.lock().await.transaction_sql(|s,tx|{
                    fleet(s,&q.fleet_id)?;tx.execute_batch(outbound::SCHEMA)?;
                    let (records,pending,bytes):(u64,u64,u64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(acked IS NULL),0),COALESCE(SUM(CASE WHEN acked IS NULL THEN bytes ELSE 0 END),0) FROM fleet_delivery WHERE fleet=?1",[&q.fleet_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
                    Ok(json!({"queue":{"records":records,"pending":pending,"bytes":bytes}}))
                })
            }
            "palpo.fleets.set_state" => {
                self.set_registration_state(bearer, &session.token, &identity.user, args)
                    .await
            }
            "palpo.fleets.migrate" => {
                self.rotate_transport(bearer, &session.token, &identity.user, args)
                    .await
            }
            _ => Err(fail(404, "service_unavailable")),
        }
    }
    async fn set_registration_state(
        &self,
        bearer: &str,
        token: &str,
        actor: &MatrixUserId,
        args: Value,
    ) -> Result<Value> {
        let fingerprint = digest(&json!({"service":"palpo.fleets.set_state","intent":args}))?;
        let input: StateIntent = serde_json::from_value(args)?;
        if !matches!(input.action.as_str(), "pause" | "resume" | "revoke") {
            return Err(fail(400, "invalid_action"));
        }
        let id = &input.fleet_id;
        let request = input.request_id.as_str();
        let state = self.store.lock().await.read()?;
        let f = fleet(&state, id)?;
        connections::view(&state, &Workflows::load(&state)?, id, actor, true, now_ms())?;
        if let Some(operation) = checked_operation(&state, request, &fingerprint)? {
            if operation["state"] == "done" {
                return self.admin_result(id, actor, &operation).await;
            }
        } else {
            no_pending(f)?;
            if f["state"] == "revoked" {
                return Err(fail(409, "fleet_revoked"));
            }
            if f["installation"] != "installed" {
                return Err(fail(409, "fleet_inactive"));
            }
            self.installed_registration(f, token).await?;
            if !self.authenticate(bearer).await?.2.admin {
                return Err(fail(403, "admin_required"));
            }
            // Fence local admissions/delivery before the first upstream write.
            // A lost reply keeps this fence until this exact operation resumes.
            self.store.lock().await.transaction(|s|{
                no_pending(fleet(s,id)?)?;
                operations(s)?.insert(request.into(),json!({"requestId":request,"digest":fingerprint,"fleetId":id,"actor":actor,"kind":"state","action":input.action,"state":"pending"}));
                let f=&mut s["fleets"][id];f["pendingAdminOperation"]=json!(request);f["localTaskStop"]=json!("unconfirmed");
                f["state"]=json!(match input.action.as_str(){"pause"=>"paused","revoke"=>"revoked",_=>"resuming"});
                association_status(s,id,match input.action.as_str(){"pause"=>"suspended","revoke"=>"revoked",_=>"configuring"})?;
                audit(s,actor,&format!("fleet.{}",input.action),id,request,"pending")
            })?;
        }
        let result = async {
            let state = self.store.lock().await.read()?;
            let f = fleet(&state, id)?;
            let desired = input.action != "resume";
            let actual = self.installed_registration(f, token).await?;
            if actual["disabled"] != desired {
                if !self.authenticate(bearer).await?.2.admin {
                    return Err(fail(403, "admin_required"));
                }
                self.matrix
                    .segments(
                        Method::POST,
                        &[
                            "_palpo",
                            "admin",
                            "v1",
                            "appservices",
                            id,
                            if desired { "disable" } else { "enable" },
                        ],
                        token,
                        None,
                        Some(&json!({})),
                    )
                    .await?;
            }
            let actual = self.installed_registration(f, token).await?;
            if actual["disabled"] != desired {
                return Err(fail(502, "state_unverified"));
            }
            if !self.authenticate(bearer).await?.2.admin {
                return Err(fail(403, "admin_required"));
            }
            self.store.lock().await.transaction(|s| {
                let f = &mut s["fleets"][id];
                f["state"] = json!(match input.action.as_str() {
                    "pause" => "paused",
                    "revoke" => "revoked",
                    _ => "pending_connection",
                });
                f["pendingAdminOperation"] = Value::Null;
                f["probe"] = Value::Null;
                f["lastError"] = Value::Null;
                f["revocationScope"] = if input.action == "revoke" {
                    json!("appservice_credentials_only")
                } else {
                    Value::Null
                };
                association_status(
                    s,
                    id,
                    match input.action.as_str() {
                        "pause" => "suspended",
                        "revoke" => "revoked",
                        _ => "verifying",
                    },
                )?;
                s["fleetAdminOperations"][request]["state"] = json!("done");
                s["fleetAdminOperations"][request]["finishedAt"] = json!(now_ms());
                audit(
                    s,
                    actor,
                    &format!("fleet.{}", input.action),
                    id,
                    request,
                    "done",
                )?;
                Ok(s["fleetAdminOperations"][request].clone())
            })
        }
        .await;
        self.finish_admin_result(id, request, actor, result).await
    }
    async fn rotate_transport(
        &self,
        bearer: &str,
        token: &str,
        actor: &MatrixUserId,
        args: Value,
    ) -> Result<Value> {
        let fingerprint = digest(&json!({"service":"palpo.fleets.migrate","intent":args}))?;
        let input: Rotation = serde_json::from_value(args)?;
        let id = &input.fleet_id;
        let request = input.request_id.as_str();
        let transport = self
            .transport_origin
            .as_ref()
            .ok_or_else(|| fail(501, "outbound_unconfigured"))?;
        let relay = self
            .relay_origin
            .as_ref()
            .ok_or_else(|| fail(501, "outbound_unconfigured"))?;
        let state = self.store.lock().await.read()?;
        let f = fleet(&state, id)?;
        connections::view(&state, &Workflows::load(&state)?, id, actor, true, now_ms())?;
        let plan = if let Some(plan) = checked_operation(&state, request, &fingerprint)? {
            if plan["state"] == "done" {
                return self.admin_result(id, actor, &plan).await;
            }
            plan
        } else {
            no_pending(f)?;
            if !matches!(f["state"].as_str(), Some("ready" | "pending_connection"))
                || f["installation"] != "installed"
            {
                return Err(fail(409, "fleet_inactive"));
            }
            if f["transport"]["mode"] == "outbound" && !input.rotate {
                return self
                    .admin_result(
                        id,
                        actor,
                        &json!({"requestId":request,"state":"done","unchanged":true}),
                    )
                    .await;
            }
            let actual = self.installed_registration(f, token).await?;
            if actual["disabled"] == true {
                return Err(fail(409, "registration_disabled"));
            }
            let generation = f["transport"]["generation"]
                .as_u64()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|n| *n < 9_007_199_254_740_991)
                .ok_or_else(|| fail(409, "generation_exhausted"))?;
            let mut desired = f["registration"].clone();
            desired["url"] = json!(format!("{relay}/api/relay/v2/{id}"));
            let plan = json!({"requestId":request,"digest":fingerprint,"fleetId":id,"actor":actor,"kind":"rotation","rotate":input.rotate,"state":"pending","previousRegistration":f["registration"],"previousGeneration":f["transport"]["generation"],"registration":desired,
                "transport":{"mode":"outbound","url":format!("{transport}/api/fleet/v2/{id}"),"token":secret(),"generation":generation}});
            if !self.authenticate(bearer).await?.2.admin {
                return Err(fail(403, "admin_required"));
            }
            self.store.lock().await.transaction(|s| {
                no_pending(fleet(s, id)?)?;
                operations(s)?.insert(request.into(), plan.clone());
                s["fleets"][id]["pendingAdminOperation"] = json!(request);
                s["fleets"][id]["state"] = json!("rotating");
                association_status(s, id, "configuring")?;
                audit(s, actor, "fleet.outbound.migrate", id, request, "pending")
            })?;
            plan
        };
        let result=async {
            let actual=self.matrix.segments(Method::GET,&["_palpo","admin","v1","appservices",id],token,None,None).await?;
            if actual["disabled"]==true{return Err(fail(409,"registration_disabled"));}
            if !registration_matches(&actual,&plan["registration"]) {
                if !registration_matches(&actual,&plan["previousRegistration"]){return Err(fail(409,"registration_drift"));}
                if !self.authenticate(bearer).await?.2.admin{return Err(fail(403,"admin_required"));}
                self.matrix.segments(Method::PUT,&["_palpo","admin","v1","appservices",id,"url"],token,None,Some(&json!({"url":plan["registration"]["url"],"expected_url":actual["url"]}))).await?;
            }
            let verified=self.matrix.segments(Method::GET,&["_palpo","admin","v1","appservices",id],token,None,None).await?;
            if verified["disabled"]==true || !registration_matches(&verified,&plan["registration"]){return Err(fail(409,"registration_drift"));}
            if !self.authenticate(bearer).await?.2.admin{return Err(fail(403,"admin_required"));}
            self.store.lock().await.transaction_sql(|s,tx|{
                tx.execute_batch(outbound::SCHEMA)?;
                let generation=plan["transport"]["generation"].as_u64().unwrap();
                if let Some(old)=plan["previousGeneration"].as_u64(){
                    tx.execute("UPDATE fleet_delivery SET generation=?1,consumer=NULL,token=NULL,expires=NULL WHERE fleet=?2 AND generation=?3 AND acked IS NULL AND kind!='probe'",params![generation,id,old])?;
                }
                let f=&mut s["fleets"][id];f["registration"]=plan["registration"].clone();f["transport"]=plan["transport"].clone();f["callbackUrl"]=Value::Null;
                f["state"]=json!("pending_connection");f["probe"]=Value::Null;f["pendingAdminOperation"]=Value::Null;f["lastError"]=Value::Null;
                let mut w=Workflows::load(s)?;
                for record in w.outbox.values_mut(){
                    if record["command"]["context"]["serverEngagementId"]==id.as_str(){record["transportGeneration"]=json!(generation);}
                }
                for record in w.agent_controls.values_mut(){
                    if record["command"]["context"]["serverEngagementId"]==id.as_str(){record["transportGeneration"]=json!(generation);}
                }
                for record in w.project_retries.values_mut(){
                    if record["command"]["context"]["serverEngagementId"]==id.as_str(){record["transportGeneration"]=json!(generation);}
                }
                w.save(s)?;
                association_status(s,id,"verifying")?;
                s["fleetAdminOperations"][request]["state"]=json!("done");s["fleetAdminOperations"][request]["finishedAt"]=json!(now_ms());
                audit(s,actor,"fleet.outbound.migrate",id,request,"done")?;Ok(s["fleetAdminOperations"][request].clone())
            })
        }.await;
        self.finish_admin_result(id, request, actor, result).await
    }
    async fn finish_admin_result(
        &self,
        id: &str,
        request: &str,
        actor: &MatrixUserId,
        result: Result<Value>,
    ) -> Result<Value> {
        match result {
            Ok(operation) => self.admin_result(id, actor, &operation).await,
            Err(error) => {
                self.store.lock().await.transaction(|s| {
                    s["fleets"][id]["lastError"] = json!(error.code);
                    s["fleetAdminOperations"][request]["lastError"] = json!(error.code);
                    Ok(())
                })?;
                Err(error)
            }
        }
    }
    async fn admin_result(
        &self,
        id: &str,
        actor: &MatrixUserId,
        operation: &Value,
    ) -> Result<Value> {
        let state = self.store.lock().await.read()?;
        let w = Workflows::load(&state)?;
        Ok(
            json!({"fleet":connections::view(&state,&w,id,actor,true,now_ms())?,"operation":{"requestId":operation["requestId"],"state":operation["state"]}}),
        )
    }
}
