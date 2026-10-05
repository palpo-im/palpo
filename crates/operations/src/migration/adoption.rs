//! Explicit local-owner receipt import. Legacy records remain immutable evidence;
//! native adoption grants capacity and current delegation governs future actions.
use palpo_hagency_contract::*;

use super::*;
use crate::workflow::{Action, Request, Resource, Workflows};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Adoption {
    pub version: u8,
    pub id: String,
    pub inventory: Inventory,
    pub native_receipt: Value,
    /// Undecided legacy requests receive an explicit grant mapping, never an approval.
    pub pending_agents: BTreeMap<String, ResourceAllocationId>,
}

fn string<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str()
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .ok_or_else(|| fail(409, "legacy_binding_invalid"))
}
fn timestamp(v: &Value, fallback: u64) -> u64 {
    v.as_u64()
        .or_else(|| {
            v.as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .and_then(|d| u64::try_from(d.timestamp_millis()).ok())
        })
        .unwrap_or(fallback)
}
fn action_id(actor: &MatrixUserId, request: &RequestId) -> Result<String> {
    Ok(format!(
        "action_{}",
        &digest(&json!({"actor":actor,"requestId":request}))?[..32]
    ))
}
fn insert(
    w: &mut Workflows,
    action: Action,
    definition: Value,
    provenance: Value,
    now: u64,
) -> Result<()> {
    if w.actions.contains_key(&action.id)
        || w.actions.len() >= 10000
        || digest(&definition)? != String::from(action.request.definition_digest().clone())
    {
        return Err(fail(409, "legacy_action_conflict"));
    }
    let id = action.id.clone();
    w.definitions.insert(digest(&definition)?, definition);
    w.legacy_sources.insert(id.clone(), provenance);
    w.actions.insert(id.clone(), action);
    w.notify(&id, now)
}
fn legacy_agent(state: &Value, fleet: &str, request: &str) -> Result<(String, Value, Value)> {
    let key = format!("{fleet}:{request}");
    let old = state["requests"][&key].clone();
    if old["id"] != key || old["fleetId"] != fleet || old["requestId"] != request {
        return Err(fail(409, "legacy_request_binding_invalid"));
    }
    let mut definition = old["payload"].clone();
    if !definition.is_object() {
        return Err(fail(409, "legacy_definition_missing"));
    }
    definition["sourceEventId"] = old["sourceEventId"].clone();
    string(&definition, "sourceEventId")?;
    Ok((key, old, definition))
}
fn validate_agent(
    w: &Workflows,
    request: &AgentRequest,
    definition: &Value,
    old: &Value,
    fleet: &str,
) -> Result<()> {
    let project = w
        .authority
        .projects
        .get(request.project_id.as_str())
        .ok_or_else(|| fail(409, "legacy_project_missing"))?;
    let resource = w
        .authority
        .resources
        .get(request.resource_allocation_id.as_str())
        .ok_or_else(|| fail(409, "legacy_resource_missing"))?;
    if request.server_engagement_id.as_str() != fleet
        || request.project_owner != project.owner
        || request.project_revision != project.revision
        || project.server_engagement_id.as_str() != fleet
        || !project
            .resource_allocations
            .contains(&request.resource_allocation_id)
        || resource.server_engagement_id.as_str() != fleet
        || !resource.eligible_managers.contains(&project.owner)
        || old["projectId"] != request.project_id.as_str()
        || old["requesterMxid"] != request.requester.as_str()
        || definition["ownerMxid"] != request.project_owner.as_str()
        || definition["requesterMxid"] != request.requester.as_str()
        || definition["targetProjectId"] != request.project_id.as_str()
        || definition["fleetId"] != fleet
        || definition["requestId"] != request.id.as_str()
        || definition["requestedTokens"] != json!(request.requested_tokens)
        || definition["agentDefinition"]["resourceId"]
            != w.resource_details[request.resource_allocation_id.as_str()]["resourceId"]
    {
        return Err(fail(409, "legacy_agent_binding_invalid"));
    }
    Ok(())
}

impl Store {
    pub fn adopt_legacy(
        &mut self,
        plan: &Adoption,
        server: &ServerName,
        now: u64,
    ) -> Result<Value> {
        if plan.version != 1
            || plan.id.is_empty()
            || plan.id.len() > 80
            || !plan
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || plan.pending_agents.len() > 1000
        {
            return Err(fail(400, "invalid_legacy_adoption"));
        }
        let fingerprint = digest(&serde_json::to_value(plan)?)?;
        self.transaction_sql(|state, tx| {
            let mut w = Workflows::load(state)?;
            if let Some(old)=w.legacy_adoptions.get(&plan.id) {
                if old["digest"]!=fingerprint { return Err(fail(409,"migration_id_conflict")); }
                return Ok(old["receipt"].clone());
            }
            let transferred: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_migrations')",[],|r|r.get(0))?;
            if !transferred { return Err(fail(409,"workflow_handoff_required")); }
            if inventory(state,tx)?!=plan.inventory { return Err(fail(409,"migration_source_changed")); }
            let native=&plan.native_receipt;
            let fleet=string(native,"serverEngagementId")?;
            let authority=w.authority.engagements.get(fleet).ok_or_else(||fail(409,"engagement_not_registered"))?;
            if native["version"]!=1 || native["registrationGeneration"]!=json!(authority.registration_generation)
                || native["delegationRevision"]!=json!(authority.delegation_revision) || native["resourceOwner"]!=authority.owner.as_str()
                || authority.state!=EngagementState::Verified || !authority.coordinator_approval_v1 || authority.delegation_expires_at_ms<=now
                || state["fleets"][fleet]["registrationGeneration"]!=json!(authority.registration_generation)
                || state["fleets"][fleet]["ownerMxid"]!=authority.owner.as_str() {
                return Err(fail(409,"migration_authority_changed"));
            }
            let _:DefinitionDigest=string(native,"sourceDigest")?.to_owned().try_into()?;
            let _:CommandId=string(native,"id")?.to_owned().try_into()?;
            let mut snapshot=w.authority.clone();
            for row in native["resources"].as_array().filter(|v|!v.is_empty()&&v.len()<=64).ok_or_else(||fail(409,"native_resources_missing"))? {
                let id=string(row,"id")?;
                let resource:Resource=serde_json::from_value(json!({"id":id,"serverEngagementId":row["serverEngagementId"],"revision":row["revision"],"allocatedTokens":row["allocatedTokens"],"eligibleManagers":row["eligibleManagers"]}))?;
                if resource.server_engagement_id.as_str()!=fleet || snapshot.resources.contains_key(id) { return Err(fail(409,"legacy_resource_conflict")); }
                string(row,"resourceId")?;
                // Native adoption includes the actual accounting period; a hostname
                // or old catalog record cannot select a replacement budget.
                string(row,"period")?; string(row,"periodKey")?;
                w.resource_details.insert(id.into(),json!({"resourceId":row["resourceId"],"period":row["period"],"periodKey":row["periodKey"]}));
                snapshot.resources.insert(id.into(),resource);
            }
            let projects=native["projects"].as_array().filter(|v|v.len()<=1000).ok_or_else(||fail(409,"native_projects_missing"))?;
            for row in projects {
                let grant:ProjectGrant=serde_json::from_value(row["grant"].clone())?;
                if grant.server_engagement_id.as_str()!=fleet || grant.state!=ProjectState::Ready || snapshot.projects.contains_key(grant.project_id.as_str()) {
                    return Err(fail(409,"legacy_project_conflict"));
                }
                snapshot.projects.insert(grant.project_id.as_str().into(),grant);
            }
            w.import_authority(snapshot,server)?;
            let mut mapped=Vec::new();
            for row in projects {
                let grant:ProjectGrant=serde_json::from_value(row["grant"].clone())?;
                let old=&state["projects"][grant.project_id.as_str()];
                let definition=&row["definition"];
                if old["fleetId"]!=fleet || old["ownerMxid"]!=grant.owner.as_str() || old["roomId"]!=definition["roomId"]
                    || old["ownerDmRoomId"]!=definition["ownerDmRoomId"] || old["name"]!=definition["name"] {
                    return Err(fail(409,"legacy_project_binding_invalid"));
                }
                let old_action=&state["actionInbox"]["records"][old["requestId"].as_str().unwrap_or_default()];
                let request_id:RequestId=string(old,"requestId")?.to_owned().try_into()?;
                let id=if old_action.is_object() {
                    if old_action["kind"]!="project" || old_action["state"]!="approved" || old_action["ownerMxid"]!=grant.owner.as_str()
                        || old_action["result"]["projectId"]!=grant.project_id.as_str() || !old_action["decision"].is_object() { return Err(fail(409,"legacy_project_decision_invalid")); }
                    string(old_action,"id")?.to_owned()
                } else { action_id(&grant.owner,&request_id)? };
                let request:ProjectRequest=serde_json::from_value(json!({"id":request_id,"revision":grant.revision,"serverEngagementId":fleet,"projectId":grant.project_id,
                    "owner":grant.owner,"requester":grant.owner,"definitionDigest":digest(definition)?,"resourceAllocations":grant.resource_allocations}))?;
                let decision=old_action.get("decision").cloned();
                insert(&mut w,Action{id:id.clone(),request:Request::Project(request),state:"approved".into(),revision:old_action["revision"].as_u64().unwrap_or(1),
                    created_at:timestamp(&old["createdAt"],now),updated_at:now,decision,execution:"ready".into()},definition.clone(),
                    json!({"migrationId":plan.id,"nativeReceiptId":native["id"],"source":"projects","sourceId":grant.project_id,"digest":digest(old)?,"originalAction":old_action}),now)?;
                mapped.push(id);
            }
            for row in native["agents"].as_array().filter(|v|v.len()<=1000).ok_or_else(||fail(409,"native_agents_missing"))? {
                let request:AgentRequest=serde_json::from_value(row["request"].clone())?;
                let (key,old,definition)=legacy_agent(state,fleet,request.id.as_str())?;
                // Native typed admission normalizes defaults. Compare the frozen
                // request fields, retaining the full native definition and hash.
                for field in ["v","fleetId","requestId","requesterMxid","sourceRoomId","sourceEventId","targetProjectId","targetRoomId","ownerMxid","ownerDmRoomId","role","requestedTokens","agentDefinition"] {
                    if row["definition"][field]!=definition[field] { return Err(fail(409,"legacy_definition_conflict")); }
                }
                validate_agent(&w,&request,&definition,&old,fleet)?;
                let agent=string(row,"agentAllocationId")?;
                if row["decisionSource"]!="legacy_native_receipts" || old["provider"]["engagementId"]!=agent {
                    return Err(fail(409,"legacy_native_decision_missing"));
                }
                let allocated:Tokens=serde_json::from_value(row["allocatedTokens"].clone())?;
                let retained:Tokens=serde_json::from_value(row["retainedTokens"].clone())?;
                if retained<allocated { return Err(fail(409,"legacy_accounting_invalid")); }
                if !row["consumedTokens"].is_null() { let consumed:Tokens=serde_json::from_value(row["consumedTokens"].clone())?; if retained<consumed {return Err(fail(409,"legacy_accounting_invalid"));} }
                let id=action_id(&request.requester,&request.id)?;
                insert(&mut w,Action{id:id.clone(),request:Request::Agent(request),state:"approved".into(),revision:1,
                    created_at:timestamp(&old["createdAt"],now),updated_at:now,decision:None,execution:"unknown".into()},row["definition"].clone(),
                    json!({"migrationId":plan.id,"nativeReceiptId":native["id"],"source":"requests","sourceId":key,"digest":digest(&old)?,"agentAllocationId":agent,"native":row}),now)?;
                // Historical observations retain their timestamps. They cannot
                // grant readiness; fresh authenticated Matrix checks still apply.
                let mut observed=old["provider"].clone();
                observed["ready"]=json!(false); observed["bound"]=json!(false);
                observed["receivedAtMs"]=json!(0); observed["allocatedTokens"]=json!(allocated);
                w.observations.insert(id.clone(),observed);
                mapped.push(id);
            }
            for (key,grant_id) in &plan.pending_agents {
                let source=&state["requests"][key];
                let (actual,old,definition)=legacy_agent(state,fleet,string(source,"requestId")?)?;
                if actual!=*key || !matches!(old["state"].as_str(),Some("pending"|"sent"|"sending"))
                    || old["provider"]["allocatedTokens"].as_u64().is_some_and(|n|n>0) || old["provider"]["ready"]==true {
                    return Err(fail(409,"legacy_request_already_decided"));
                }
                let project=w.authority.projects.get(string(&old,"projectId")?).ok_or_else(||fail(409,"legacy_project_missing"))?;
                let request:AgentRequest=serde_json::from_value(json!({"id":old["requestId"],"revision":1,"serverEngagementId":fleet,"projectId":old["projectId"],"projectRevision":project.revision,
                    "resourceAllocationId":grant_id,"projectOwner":project.owner,"requester":old["requesterMxid"],"definitionDigest":digest(&definition)?,"requestedTokens":definition["requestedTokens"]}))?;
                validate_agent(&w,&request,&definition,&old,fleet)?;
                let actor=request.requester.clone();
                let id=action_id(&actor,&request.id)?;
                if w.actions.contains_key(&id) { return Err(fail(409,"legacy_action_conflict")); }
                w.submit_definition(Request::Agent(request),definition,&actor,now)?;
                w.actions.get_mut(&id).unwrap().created_at=timestamp(&old["createdAt"],now);
                w.legacy_sources.insert(id.clone(),json!({"migrationId":plan.id,"source":"requests","sourceId":key,"digest":digest(&old)?}));
                mapped.push(id);
            }
            let receipt=json!({"version":1,"id":plan.id,"nativeReceiptDigest":digest(native)?,"sourceDigest":plan.inventory.source_digest,"actions":mapped,"committedAtMs":now});
            w.legacy_adoptions.insert(plan.id.clone(),json!({"digest":fingerprint,"receipt":receipt,"nativeReceipt":native}));
            w.save(state)?;
            Ok(receipt)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(store: &mut Store) -> (ServerName, Adoption) {
        let server: ServerName = "example.test".to_owned().try_into().unwrap();
        let definition = json!({"v":1,"fleetId":"fleet_one","requestId":"old_agent","requesterMxid":"@manager:example.test","sourceRoomId":"!reception:example.test","sourceEventId":"$original","targetProjectId":"project_one","targetRoomId":"!project:example.test","ownerMxid":"@manager:example.test","ownerDmRoomId":"!private:example.test","role":"worker","requestedTokens":100,"agentDefinition":{"name":"Original","resourceId":"resource_old"}});
        store.transaction(|s| {
            s["fleets"]=json!({"fleet_one":{"id":"fleet_one","registrationGeneration":1,"ownerMxid":"@provider:example.test","representativeMxid":"@fleet_one_bot:example.test","transport":{"generation":1}}});
            s["projects"]=json!({"project_one":{"id":"project_one","requestId":"action_original","name":"Original project","fleetId":"fleet_one","ownerMxid":"@manager:example.test","roomId":"!project:example.test","ownerDmRoomId":"!private:example.test","createdAt":20}});
            s["actionInbox"] = json!({"records":{"action_original":{"id":"action_original","requestId":"old_project","kind":"project","ownerMxid":"@manager:example.test","state":"approved","execution":"done","revision":2,"result":{"projectId":"project_one"},"decision":{"by":"@original_admin:example.test","at":21,"reason":"Original verdict"}}}});
            for id in ["old_agent","old_pending"] {
                let mut payload=definition.clone(); payload["requestId"]=json!(id); payload.as_object_mut().unwrap().remove("sourceEventId");
                s["requests"][format!("fleet_one:{id}")]=json!({"id":format!("fleet_one:{id}"),"requestId":id,"fleetId":"fleet_one","projectId":"project_one","requesterMxid":"@manager:example.test","sourceEventId":"$original","payload":payload,
                    "state":if id=="old_agent" {"active"}else{"pending"},"createdAt":30,
                    "provider":if id=="old_agent" {json!({"engagementId":"native_original","allocatedTokens":100,"state":"active","ready":true})}else{json!({"state":"pending"})}});
            }
            let mut w=Workflows::default();
            w.import_authority(serde_json::from_value(json!({"engagements":{"fleet_one":{"id":"fleet_one","server":"example.test","owner":"@provider:example.test","coordinator":"@coordinator:example.test","registrationGeneration":1,"delegationRevision":1,"delegationExpiresAtMs":1000000,"state":"verified","allowSelfApproval":false,"coordinatorApprovalV1":true}},"resources":{},"projects":{}}))?,&server)?;
            w.save(s)
        }).unwrap();
        let inventory = store.migration_inventory().unwrap();
        store
            .handoff_legacy(
                &Handoff {
                    version: 1,
                    id: "handoff".into(),
                    inventory,
                    native_audit_digest: "a".repeat(64),
                },
                1000,
            )
            .unwrap();
        let receipt = json!({"version":1,"id":"native_adoption","sourceDigest":"a".repeat(64),"serverEngagementId":"fleet_one","registrationGeneration":1,"delegationRevision":1,"resourceOwner":"@provider:example.test","acceptedAtMs":999,
            "resources":[{"id":"grant_one","serverEngagementId":"fleet_one","resourceId":"resource_old","revision":1,"allocatedTokens":150,"eligibleManagers":["@manager:example.test"],"period":"day","periodKey":"1970-01-01"}],
            "projects":[{"grant":{"projectId":"project_one","serverEngagementId":"fleet_one","revision":1,"owner":"@manager:example.test","resourceAllocations":["grant_one"],"state":"ready"},"definition":{"name":"Original project","roomId":"!project:example.test","ownerDmRoomId":"!private:example.test"}}],
            "agents":[{"agentAllocationId":"native_original","request":{"id":"old_agent","revision":1,"serverEngagementId":"fleet_one","projectId":"project_one","projectRevision":1,"resourceAllocationId":"grant_one","projectOwner":"@manager:example.test","requester":"@manager:example.test","definitionDigest":digest(&definition).unwrap(),"requestedTokens":100},"definition":definition,"allocatedTokens":100,"retainedTokens":100,"consumedTokens":null,"decisionSource":"legacy_native_receipts"}]});
        let plan = Adoption {
            version: 1,
            id: "adopt_one".into(),
            inventory: store.migration_inventory().unwrap(),
            native_receipt: receipt,
            pending_agents: BTreeMap::from([(
                "fleet_one:old_pending".into(),
                "grant_one".to_owned().try_into().unwrap(),
            )]),
        };
        (server, plan)
    }

    #[test]
    fn adoption_preserves_sources_fences_mismatches_and_keeps_pending_requests_undecided() {
        let mut store = Store::memory().unwrap();
        let (server, plan) = fixture(&mut store);
        let before = store.read().unwrap();
        for changed in [
            "resourceOwner",
            "registrationGeneration",
            "sourceEventId",
            "nativeAgent",
        ] {
            let mut bad = plan.clone();
            match changed {
                "resourceOwner" => bad.native_receipt[changed] = json!("@other:example.test"),
                "registrationGeneration" => bad.native_receipt[changed] = json!(2),
                "sourceEventId" => {
                    bad.native_receipt["agents"][0]["definition"][changed] = json!("$different")
                }
                _ => bad.native_receipt["agents"][0]["agentAllocationId"] = json!("another_agent"),
            }
            assert!(
                store.adopt_legacy(&bad, &server, 1000).is_err(),
                "{changed}"
            );
            assert_eq!(store.read().unwrap(), before);
        }
        let receipt = store.adopt_legacy(&plan, &server, 1000).unwrap();
        let after = store.read().unwrap();
        for key in ["fleets", "projects", "requests", "audit"] {
            assert_eq!(before[key], after[key]);
        }
        let mut w = Workflows::load(&after).unwrap();
        assert!(
            w.outbox.is_empty(),
            "Migration must never provision another agent"
        );
        assert_eq!(
            w.actions["action_original"].decision.as_ref().unwrap()["by"],
            "@original_admin:example.test"
        );
        assert_eq!(w.actions["action_original"].revision, 2);
        let owner: MatrixUserId = "@manager:example.test".to_owned().try_into().unwrap();
        let old = action_id(&owner, &"old_agent".to_owned().try_into().unwrap()).unwrap();
        let pending = action_id(&owner, &"old_pending".to_owned().try_into().unwrap()).unwrap();
        assert_eq!(w.actions[&old].state, "approved");
        assert_eq!(w.actions[&old].execution, "unknown");
        assert!(
            w.actions[&old].decision.is_none(),
            "Do not invent a coordinator verdict"
        );
        assert_eq!(w.observations[&old]["engagementId"], "native_original");
        assert_eq!(w.observations[&old]["ready"], false);
        assert!(w.legacy_sources[&old]["native"]["consumedTokens"].is_null());
        assert_eq!(w.actions[&pending].state, "requested");
        let mut status = plan.native_receipt["agents"][0]["definition"].clone();
        for (key,value) in json!({"engagementId":"native_original","state":"active","allocatedTokens":100,"agentMxid":"@fleet_one_native_original:example.test","bound":true,"ready":true,"observedAt":"1970-01-01T00:00:01.001Z","fulfillment":{"phase":"complete","incomplete":false}}).as_object().unwrap() { status[key]=value.clone(); }
        let update =
            json!({"v":2,"generation":1,"sequence":1,"heartbeat":true,"statuses":[status]});
        let mut wrong = update.clone();
        wrong["statuses"][0]["engagementId"] = json!("another_agent");
        assert!(
            store
                .transaction_sql(|s, tx| crate::updates::apply(
                    s,
                    tx,
                    s["fleets"]["fleet_one"].clone(),
                    &wrong,
                    None,
                    &server,
                    1001
                ))
                .is_err()
        );
        store
            .transaction_sql(|s, tx| {
                crate::updates::apply(
                    s,
                    tx,
                    s["fleets"]["fleet_one"].clone(),
                    &update,
                    None,
                    &server,
                    1001,
                )
            })
            .unwrap();
        w = Workflows::load(&store.read().unwrap()).unwrap();
        assert_eq!(w.actions[&old].execution, "ready");
        assert!(
            w.outbox.is_empty(),
            "Fresh observations do not repeat approval"
        );

        let request =
            serde_json::to_value(&w.actions[&pending].request).unwrap()["request"].clone();
        let command = json!({"context":{"version":1,"commandId":"approve_pending","serverEngagementId":"fleet_one","registrationGeneration":1,"delegationRevision":1,"actor":"@coordinator:example.test","issuedAtMs":1001,"expiresAtMs":2000},"request":request,"allocatedTokens":50});
        assert!(
            w.approve(
                &pending,
                command.clone(),
                &"@admin:example.test".to_owned().try_into().unwrap(),
                1001
            )
            .is_err()
        );
        w.approve(
            &pending,
            command,
            &"@coordinator:example.test".to_owned().try_into().unwrap(),
            1001,
        )
        .unwrap();
        store.transaction(|s| w.save(s)).unwrap();
        let later = store.read().unwrap();
        assert_eq!(
            store.adopt_legacy(&plan, &server, 2000000).unwrap(),
            receipt
        );
        assert_eq!(store.read().unwrap(), later);
        assert_eq!(w.outbox.len(), 1);
        let mut changed = plan;
        changed.native_receipt["agents"][0]["allocatedTokens"] = json!(99);
        assert!(store.adopt_legacy(&changed, &server, 1001).is_err());
    }
}
