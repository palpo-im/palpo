//! Native port of web-admin/lib/outbound.mjs's durable leased delivery lane.
//! Custody acknowledgement never marks an agent ready or refunds a reservation.
use chrono::{DateTime, SecondsFormat};
use palpo_hagency_contract::canonical::{encode_transport, transport_digest};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;

use crate::{Result, digest, fail, secret};

pub const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS fleet_delivery (
 ordinal INTEGER PRIMARY KEY AUTOINCREMENT, fleet TEXT NOT NULL, generation INTEGER NOT NULL,
 lane TEXT NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL, digest TEXT NOT NULL,
 payload TEXT, bytes INTEGER NOT NULL, consumer TEXT, token TEXT, expires INTEGER, acked INTEGER,
 UNIQUE(fleet,generation,lane,id));
 CREATE INDEX IF NOT EXISTS fleet_delivery_pending ON fleet_delivery(fleet,generation,lane,acked,ordinal);";

#[derive(Clone, Copy)]
pub struct Limits {
    pub lease_ms: u64,
    pub records: u64,
    pub pending: u64,
    pub bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            lease_ms: 30000,
            records: 10000,
            pending: 1000,
            bytes: 16 * 1024 * 1024,
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 255 && !id.chars().any(char::is_control)
}
fn lane(value: &str) -> bool {
    value == "matrix" || value == "work"
}
fn same_secret(left: &str, right: &str) -> Result<bool> {
    // Hashes have fixed length, including when credentials have different sizes.
    let a = digest(&json!(left))?;
    let b = digest(&json!(right))?;
    Ok(bool::from(a.as_bytes().ct_eq(b.as_bytes())))
}
pub fn authenticate(
    state: &Value,
    id: &str,
    token: &str,
    generation: Option<u64>,
    relay: bool,
) -> Result<Value> {
    let fleet = &state["fleets"][id];
    let expected = if relay {
        fleet.pointer("/registration/hs_token")
    } else {
        fleet.pointer("/transport/token")
    };
    if token.is_empty()
        || token.len() > 8192
        || fleet["id"] != id
        || fleet.pointer("/transport/mode") != Some(&json!("outbound"))
        || !matches!(
            fleet["state"].as_str(),
            Some("pending_connection" | "ready")
        )
        || fleet["installation"] != "installed"
        || !same_secret(expected.and_then(Value::as_str).unwrap_or_default(), token)?
    {
        return Err(fail(401, "transport_unauthorized"));
    }
    if !relay
        && (generation.is_none()
            || generation == Some(0)
            || generation
                != fleet
                    .pointer("/transport/generation")
                    .and_then(Value::as_u64))
    {
        return Err(fail(409, "generation_conflict"));
    }
    Ok(fleet.clone())
}

pub fn enqueue(
    tx: &Transaction<'_>,
    fleet: &Value,
    lane_name: &str,
    kind: &str,
    id: &str,
    payload: &Value,
    limits: Limits,
) -> Result<()> {
    tx.execute_batch(SCHEMA)?;
    if !valid_id(id)
        || !lane(lane_name)
        || !payload.is_object()
        || (lane_name == "matrix" && kind != "transaction")
        || (lane_name == "work" && !matches!(kind, "request" | "probe"))
    {
        return Err(fail(400, "invalid_delivery"));
    }
    let fid = fleet["id"]
        .as_str()
        .ok_or_else(|| fail(400, "invalid_fleet"))?;
    let generation = fleet["transport"]["generation"]
        .as_u64()
        .ok_or_else(|| fail(400, "invalid_generation"))?;
    let fingerprint = transport_digest(payload)?;
    let prior: Option<String> = if lane_name == "matrix" {
        tx.query_row("SELECT digest FROM fleet_delivery WHERE fleet=?1 AND lane='matrix' AND id=?2 ORDER BY ordinal DESC LIMIT 1", params![fid,id], |r|r.get(0)).optional()?
    } else {
        tx.query_row("SELECT digest FROM fleet_delivery WHERE fleet=?1 AND generation=?2 AND lane=?3 AND id=?4", params![fid,generation,lane_name,id], |r|r.get(0)).optional()?
    };
    if let Some(prior) = prior {
        return if prior == fingerprint {
            Ok(())
        } else {
            Err(fail(409, "delivery_conflict"))
        };
    }
    let raw = encode_transport(payload)?;
    if raw.len() > 4 * 1024 * 1024 {
        return Err(fail(400, "delivery_too_large"));
    }
    let (records,pending,bytes): (u64,u64,u64) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(acked IS NULL),0),COALESCE(SUM(CASE WHEN acked IS NULL THEN bytes ELSE 0 END),0) FROM fleet_delivery WHERE fleet=?1", [fid], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if records >= limits.records
        || pending >= limits.pending
        || bytes.saturating_add(raw.len() as u64) > limits.bytes
    {
        return Err(fail(503, "queue_full"));
    }
    tx.execute("INSERT INTO fleet_delivery(fleet,generation,lane,id,kind,digest,payload,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![fid,generation,lane_name,id,kind,fingerprint,raw,raw.len()])?;
    Ok(())
}

pub fn claim(
    tx: &Transaction<'_>,
    fleet: &Value,
    lane_name: &str,
    consumer: &str,
    now: u64,
    limits: Limits,
) -> Result<Value> {
    tx.execute_batch(SCHEMA)?;
    let valid_consumer = consumer.len() == 36
        && consumer.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if !lane(lane_name) || !valid_consumer {
        return Err(fail(400, "invalid_poll"));
    }
    let generation = fleet["transport"]["generation"]
        .as_u64()
        .ok_or_else(|| fail(400, "invalid_generation"))?;
    let row: Option<(i64,String,String,String,Option<u64>)> = tx.query_row("SELECT ordinal,id,kind,payload,expires FROM fleet_delivery WHERE fleet=?1 AND generation=?2 AND lane=?3 AND acked IS NULL ORDER BY ordinal LIMIT 1",params![fleet["id"].as_str(),generation,lane_name], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((ordinal, id, kind, payload, expires)) =
        row.filter(|r| r.4.is_none_or(|until| until <= now))
    else {
        return Ok(json!({"v":2,"generation":generation,"delivery":null}));
    };
    let _ = expires;
    let until = now
        .checked_add(limits.lease_ms)
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(|| fail(400, "invalid_clock"))?;
    let expires = DateTime::from_timestamp_millis(until)
        .ok_or_else(|| fail(400, "invalid_clock"))?
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let token = secret();
    tx.execute(
        "UPDATE fleet_delivery SET consumer=?1,token=?2,expires=?3 WHERE ordinal=?4",
        params![consumer, token, until, ordinal],
    )?;
    Ok(
        json!({"v":2,"generation":generation,"delivery":{"id":id,"lane":lane_name,"token":token,"expiresAt":expires,"kind":kind,"payload":serde_json::from_str::<Value>(&payload)?}}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ack {
    pub lane: String,
    pub id: String,
    pub token: String,
}
pub fn ack(tx: &Transaction<'_>, fleet: &Value, input: &Ack, now: u64) -> Result<Value> {
    tx.execute_batch(SCHEMA)?;
    type LeaseReceipt = (i64, Option<String>, Option<u64>, Option<u64>);
    let row: Option<LeaseReceipt> = tx.query_row("SELECT ordinal,token,expires,acked FROM fleet_delivery WHERE fleet=?1 AND generation=?2 AND lane=?3 AND id=?4",params![fleet["id"].as_str(),fleet["transport"]["generation"].as_u64(),input.lane,input.id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((ordinal, token, expires, acked)) = row else {
        return Err(fail(409, "stale_lease"));
    };
    if input.token.is_empty()
        || !same_secret(token.as_deref().unwrap_or_default(), &input.token)?
        || acked.is_none() && expires.is_none_or(|until| until <= now)
    {
        return Err(fail(409, "stale_lease"));
    }
    if acked.is_none() {
        tx.execute(
            "UPDATE fleet_delivery SET acked=?1,payload=NULL WHERE ordinal=?2",
            params![now, ordinal],
        )?;
    }
    Ok(json!({"ok":true}))
}

/// Matrix transactions and exact probe evidence commit together, just as in
/// Node. Transaction-ID dedup survives transport credential generation changes.
pub fn relay(
    tx: &Transaction<'_>,
    fleet: &mut Value,
    id: &str,
    body: Value,
    limits: Limits,
) -> Result<()> {
    let events = body["events"]
        .as_array()
        .filter(|events| events.len() <= 1000 && events.iter().all(Value::is_object))
        .ok_or_else(|| fail(400, "invalid_transaction"))?;
    enqueue(
        tx,
        fleet,
        "matrix",
        "transaction",
        id,
        &json!({"transactionId":id,"body":body}),
        limits,
    )?;
    for event in events {
        let probe = &fleet["probe"];
        if probe.is_object()
            && event["type"] == "com.hagency.connection.probe.v1"
            && event["room_id"] == probe["roomId"]
            && event["sender"] == fleet["representativeMxid"]
            && event["content"]["fleetId"] == fleet["id"]
            && event["content"]["challenge"] == probe["challenge"]
            && event["event_id"].as_str().is_some_and(valid_id)
            && (probe["eventId"].is_null() || probe["eventId"] == event["event_id"])
        {
            fleet["probe"]["matrixTransactionId"] = json!(id);
            fleet["probe"]["matrixEventId"] = event["event_id"].clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    fn fleet() -> Value {
        json!({"id":"hf_test","installation":"installed","state":"ready","transport":{"mode":"outbound","generation":1,"token":"fixture-machine"},"registration":{"hs_token":"fixture-relay"}})
    }
    #[test]
    fn custody_replays_conflicts_expiry_and_ack_never_change_workflow_readiness() {
        let mut store = Store::memory().unwrap();
        let f = fleet();
        let l = Limits::default();
        let consumer = "01234567-0123-0123-0123-0123456789ab";
        let lease = store
            .transaction_sql(|state, tx| {
                state["agentState"] = json!("pending");
                enqueue(
                    tx,
                    &f,
                    "work",
                    "request",
                    "request1",
                    &json!({"agent":"a"}),
                    l,
                )?;
                enqueue(
                    tx,
                    &f,
                    "work",
                    "request",
                    "request1",
                    &json!({"agent":"a"}),
                    l,
                )?;
                claim(tx, &f, "work", consumer, 1000, l)
            })
            .unwrap();
        assert!(
            store
                .transaction_sql(|_, tx| enqueue(
                    tx,
                    &f,
                    "work",
                    "request",
                    "request1",
                    &json!({"agent":"b"}),
                    l
                ))
                .is_err()
        );
        assert!(
            store
                .transaction_sql(|_, tx| claim(tx, &f, "work", consumer, 1001, l))
                .unwrap()["delivery"]
                .is_null()
        );
        let old = Ack {
            lane: "work".into(),
            id: "request1".into(),
            token: lease["delivery"]["token"].as_str().unwrap().into(),
        };
        assert!(
            store
                .transaction_sql(|_, tx| ack(tx, &f, &old, 31000))
                .is_err()
        );
        let replacement = store
            .transaction_sql(|_, tx| claim(tx, &f, "work", consumer, 31000, l))
            .unwrap();
        assert!(
            store
                .transaction_sql(|_, tx| ack(tx, &f, &old, 31001))
                .is_err()
        );
        let current = Ack {
            token: replacement["delivery"]["token"].as_str().unwrap().into(),
            ..old
        };
        store
            .transaction_sql(|_, tx| ack(tx, &f, &current, 31001))
            .unwrap();
        store
            .transaction_sql(|_, tx| ack(tx, &f, &current, 99000))
            .unwrap();
        assert_eq!(store.read().unwrap()["agentState"], "pending");
        assert!(
            store
                .transaction_sql(|_, tx| claim(tx, &f, "work", consumer, 99000, l))
                .unwrap()["delivery"]
                .is_null()
        );
    }
    #[test]
    fn authentication_generation_and_relay_credentials_are_distinct() {
        let mut state = json!({"fleets":{"hf_test":fleet()}});
        assert!(authenticate(&state, "hf_test", "fixture-machine", Some(1), false).is_ok());
        assert!(authenticate(&state, "hf_test", "fixture-relay", Some(1), false).is_err());
        assert!(authenticate(&state, "hf_test", "fixture-machine", None, true).is_err());
        assert!(authenticate(&state, "hf_test", "fixture-machine", Some(2), false).is_err());
        state["fleets"]["hf_test"]["state"] = json!("revoked");
        assert!(authenticate(&state, "hf_test", "fixture-machine", Some(1), false).is_err());
    }
    #[test]
    fn matrix_dedup_survives_rotation_and_failed_write_rolls_back_both_tables() {
        let mut store = Store::memory().unwrap();
        let l = Limits::default();
        let mut f = fleet();
        let body = json!({"events":[]});
        store
            .transaction_sql(|state, tx| {
                relay(tx, &mut f, "txn", body.clone(), l)?;
                state["value"] = json!(1);
                Ok(())
            })
            .unwrap();
        f["transport"]["generation"] = json!(2);
        store
            .transaction_sql(|_, tx| relay(tx, &mut f, "txn", body.clone(), l))
            .unwrap();
        assert!(
            store
                .transaction_sql::<()>(|state, tx| {
                    relay(tx, &mut f, "txn2", body.clone(), l)?;
                    state["value"] = json!(2);
                    Err(fail(409, "fixture_failure"))
                })
                .is_err()
        );
        store
            .transaction_sql(|state, tx| {
                assert_eq!(state["value"], 1);
                assert_eq!(
                    tx.query_row("SELECT COUNT(*) FROM fleet_delivery", [], |r| r
                        .get::<_, u64>(0))?,
                    1
                );
                Ok(())
            })
            .unwrap();
    }
}
