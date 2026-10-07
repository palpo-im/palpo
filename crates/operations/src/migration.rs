mod adoption;
use std::collections::BTreeMap;

pub use adoption::Adoption;
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// Offline, digest-bound workflow handoff. No role or token grant is inferred
// from a legacy administrator verdict, catalog entry or hostname.
use crate::{Result, digest, fail, store::Store};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mapping {
    pub source: String,
    pub id: String,
    pub digest: String,
    pub server_engagement_id: Option<String>,
    pub project_id: Option<String>,
    pub agent_allocation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inventory {
    pub version: u8,
    pub source_digest: String,
    pub counts: BTreeMap<String, usize>,
    pub mappings: Vec<Mapping>,
    pub delivery_digest: String,
    pub delivery_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handoff {
    pub version: u8,
    pub id: String,
    pub inventory: Inventory,
    /// Operator evidence identifying the copied native database/accounting audit.
    /// Retain the audit, but never use a digest as an allocation permission.
    pub native_audit_digest: String,
}

fn identity(value: &Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 255 && !s.chars().any(char::is_control))
        .ok_or_else(|| fail(409, "legacy_identity_invalid"))?;
    Ok(Some(value.to_owned()))
}

fn rows(state: &Value, key: &str) -> Result<Vec<(String, Value)>> {
    if state[key].is_null() {
        return Ok(Vec::new());
    }
    state[key]
        .as_object()
        .map(|rows| {
            rows.iter()
                .map(|(id, row)| (id.clone(), row.clone()))
                .collect()
        })
        .ok_or_else(|| fail(409, "legacy_collection_invalid"))
}

fn inventory(state: &Value, tx: &Transaction<'_>) -> Result<Inventory> {
    let mut mappings = Vec::new();
    let mut counts = BTreeMap::new();
    for (kind, values) in [
        ("fleets", rows(state, "fleets")?),
        ("projects", rows(state, "projects")?),
        ("requests", rows(state, "requests")?),
        ("actions", rows(&state["actionInbox"], "records")?),
        ("accounts", rows(&state["accountAccess"], "requests")?),
    ] {
        counts.insert(kind.to_owned(), values.len());
        for (id, row) in values {
            if !row.is_object() || row.get("id").is_some_and(|value| value != &json!(id)) {
                return Err(fail(409, "legacy_record_binding_invalid"));
            }
            let fleet = match kind {
                "fleets" => Some(id.clone()),
                "actions" => identity(
                    row.get("result")
                        .and_then(|r| r.get("fleetId"))
                        .unwrap_or(&row["payload"]["fleetId"]),
                )?,
                _ => identity(&row["fleetId"])?,
            };
            if fleet
                .as_ref()
                .is_some_and(|fleet| state["fleets"][fleet].is_null())
            {
                return Err(fail(409, "legacy_fleet_missing"));
            }
            let project = if kind == "projects" {
                Some(id.clone())
            } else {
                identity(&row["projectId"])?
            };
            if kind == "requests"
                && (id
                    != format!(
                        "{}:{}",
                        fleet.as_deref().unwrap_or_default(),
                        row["requestId"].as_str().unwrap_or_default()
                    )
                    || project
                        .as_ref()
                        .is_none_or(|id| state["projects"][id]["fleetId"] != row["fleetId"]))
            {
                return Err(fail(409, "legacy_request_binding_invalid"));
            }
            mappings.push(Mapping {
                source: kind.into(),
                id,
                digest: digest(&row)?,
                server_engagement_id: fleet,
                project_id: project,
                agent_allocation_id: identity(&row["provider"]["engagementId"])?,
            });
        }
    }
    let mut agent_count = 0;
    for (fleet, row) in rows(state, "fleets")? {
        for (id, agent) in rows(&row, "agents")? {
            if !agent.is_object() || agent.get("id").is_some_and(|value| value != &json!(id)) {
                return Err(fail(409, "legacy_record_binding_invalid"));
            }
            mappings.push(Mapping {
                source: "agents".into(),
                id: format!("{fleet}:{id}"),
                digest: digest(&agent)?,
                server_engagement_id: Some(fleet.clone()),
                project_id: identity(&agent["projectId"])?,
                agent_allocation_id: identity(&agent["engagementId"])?,
            });
            agent_count += 1;
        }
    }
    counts.insert("agents".into(), agent_count);
    counts.insert(
        "audit".into(),
        state["audit"]
            .as_array()
            .ok_or_else(|| fail(409, "legacy_audit_invalid"))?
            .len(),
    );
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='fleet_delivery')",
        [],
        |r| r.get(0),
    )?;
    let deliveries = if exists {
        let mut query = tx.prepare("SELECT ordinal,fleet,generation,lane,id,kind,digest,payload,bytes,consumer,token,expires,acked FROM fleet_delivery ORDER BY ordinal")?;
        query
            .query_map([], |r| {
                Ok(json!([
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<String>>(10)?,
                    r.get::<_, Option<i64>>(11)?,
                    r.get::<_, Option<i64>>(12)?
                ]))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    Ok(Inventory {
        version: 1,
        source_digest: digest(state)?,
        counts,
        mappings,
        delivery_digest: digest(&json!(deliveries))?,
        delivery_count: deliveries.len(),
    })
}

impl Store {
    /// A rollback-only transaction: inventory cannot normalize or rewrite any
    /// source record, delivery lease, credential or unknown extension.
    pub fn migration_inventory(&mut self) -> Result<Inventory> {
        self.inspect_sql(inventory)
    }

    pub fn handoff_legacy(&mut self, handoff: &Handoff, now: u64) -> Result<Value> {
        if handoff.version != 1
            || handoff.id.is_empty()
            || handoff.id.len() > 80
            || !handoff
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || handoff.native_audit_digest.len() != 64
            || !handoff
                .native_audit_digest
                .bytes()
                .all(|c| c.is_ascii_hexdigit())
        {
            return Err(fail(400, "invalid_migration_handoff"));
        }
        let operation = digest(&serde_json::to_value(handoff)?)?;
        self.transaction_sql(|state, tx| {
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workflow_migrations')", [], |r|r.get(0))?;
            if exists {
                let previous: Option<(String,String)> = tx.query_row("SELECT digest,receipt FROM workflow_migrations WHERE id=?1", [&handoff.id], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some((digest, receipt)) = previous {
                    if digest != operation { return Err(fail(409, "migration_id_conflict")); }
                    return Ok(serde_json::from_str(&receipt)?);
                }
                return Err(fail(409, "workflow_authority_already_transferred"));
            }
            if inventory(state, tx)? != handoff.inventory {
                return Err(fail(409, "migration_source_changed"));
            }
            // Preserve the source document, including native agent allocation
            // IDs, old decisions and unknown extensions byte-for-byte in value.
            // The stable mapping is a provenance index, not a new approval.
            let receipt = json!({"version":1,"id":handoff.id,"sourceDigest":handoff.inventory.source_digest,
                "deliveryDigest":handoff.inventory.delivery_digest,"counts":handoff.inventory.counts,
                "mappings":handoff.inventory.mappings,"nativeAuditDigest":handoff.native_audit_digest,
                "authority":"rust","committedAtMs":now});
            tx.execute_batch("CREATE TABLE workflow_migrations(id TEXT PRIMARY KEY,digest TEXT NOT NULL,receipt TEXT NOT NULL)")?;
            crate::store::fence_legacy_writers(tx)?;
            tx.execute("INSERT INTO workflow_migrations VALUES(?1,?2,?3)", rusqlite::params![handoff.id,operation,serde_json::to_string(&receipt)?])?;
            Ok(receipt)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handoff_retains_stable_ids_and_replays_after_new_rust_work_without_reverting_it() {
        let mut store = Store::memory().unwrap();
        store.transaction(|state| {
            state["fleets"] = json!({"fleet_one":{"id":"fleet_one","registration":{"as_token":"fixture-private"},"agents":{"bot":{"id":"bot","projectId":"project_one","engagementId":"legacy_agent_allocation"}}}});
            state["projects"] = json!({"project_one":{"id":"project_one","fleetId":"fleet_one","ownerMxid":"@owner:test"}});
            state["requests"] = json!({"fleet_one:request_one":{"id":"fleet_one:request_one","requestId":"request_one","fleetId":"fleet_one","projectId":"project_one","provider":{"engagementId":"legacy_agent_allocation"},"state":"active"}});
            state["accountAccess"] = json!({"requests":{"signup":{"id":"signup","password":{"ciphertext":"fixture"}}}});
            state["unknownExtension"] = json!({"preserve":"原有记录"});
            Ok(())
        }).unwrap();
        let original = store.read().unwrap();
        let inventory = store.migration_inventory().unwrap();
        assert_eq!(inventory.counts["accounts"], 1);
        assert_eq!(inventory.counts["agents"], 1);
        assert!(inventory.mappings.iter().any(|row| row.source == "requests"
            && row.id == "fleet_one:request_one"
            && row.server_engagement_id.as_deref() == Some("fleet_one")
            && row.agent_allocation_id.as_deref() == Some("legacy_agent_allocation")));
        assert!(
            !serde_json::to_string(&inventory)
                .unwrap()
                .contains("fixture-private")
        );
        let mut handoff = Handoff {
            version: 1,
            id: "cutover_one".into(),
            inventory,
            native_audit_digest: "a".repeat(64),
        };
        handoff.inventory.delivery_count += 1;
        assert_eq!(
            store.handoff_legacy(&handoff, 100).unwrap_err().code,
            "migration_source_changed"
        );
        assert_eq!(store.read().unwrap(), original);
        handoff.inventory.delivery_count -= 1;
        let receipt = store.handoff_legacy(&handoff, 101).unwrap();
        assert_eq!(store.read().unwrap(), original);
        store
            .transaction(|state| {
                state["postCutoverDecision"] = json!({"id":"new_decision","state":"approved"});
                Ok(())
            })
            .unwrap();
        let current = store.read().unwrap();
        assert_eq!(store.handoff_legacy(&handoff, 102).unwrap(), receipt);
        assert_eq!(store.read().unwrap(), current);
        handoff.native_audit_digest = "b".repeat(64);
        assert_eq!(
            store.handoff_legacy(&handoff, 103).unwrap_err().code,
            "migration_id_conflict"
        );
        assert_eq!(store.read().unwrap(), current);
    }
}
