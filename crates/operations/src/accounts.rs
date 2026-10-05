//! Signup approval is separate from project and resource authority. Its legacy
//! IDs, sealed passwords and registration-device proof survive Rust cutover.
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use palpo_hagency_contract::{MatrixUserId, ServerName};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{Result, fail};

mod api;
mod worker;
pub(crate) use api::{Limits, router};
pub use worker::{start, tick};

impl crate::api::App {
    pub fn with_accounts(
        mut self: std::sync::Arc<Self>,
        config: Configuration,
    ) -> Result<std::sync::Arc<Self>> {
        let app = std::sync::Arc::get_mut(&mut self)
            .ok_or_else(|| fail(409, "account_configuration_locked"))?;
        app.store
            .get_mut()
            .transaction(|state| config.bind(state, app.matrix.server()))?;
        app.accounts = Some(config);
        Ok(self)
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configuration {
    pub bot_mxid: MatrixUserId,
    pub bot_token: String,
    pub admin_token: String,
    pub approvers: Vec<MatrixUserId>,
    pub password_key: String,
    pub registration_token: String,
}
fn hash(bytes: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn bytes(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(fail(503, "invalid_account_secret"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|c| {
            u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16)
                .map_err(|_| fail(503, "invalid_account_secret"))
        })
        .collect()
}
fn hex_id(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Configuration {
    pub(crate) fn bind(&self, state: &mut Value, server: &ServerName) -> Result<()> {
        if !self.bot_mxid.belongs_to(server)
            || !hex_id(&self.password_key, 64)
            || self.approvers.is_empty()
            || self.approvers.len() > 32
            || self
                .approvers
                .iter()
                .any(|a| a == &self.bot_mxid || !a.belongs_to(server))
            || [&self.bot_token, &self.admin_token, &self.registration_token]
                .iter()
                .any(|s| s.is_empty() || s.len() > 8192 || s.chars().any(char::is_control))
        {
            return Err(fail(400, "invalid_account_configuration"));
        }
        let mut approvers: Vec<&str> = self.approvers.iter().map(|a| a.as_str()).collect();
        approvers.sort();
        approvers.dedup();
        let binding = hash(serde_json::to_vec(&json!([
            self.bot_mxid,
            approvers,
            hash(&self.password_key)
        ]))?);
        if state["accountAccess"]["binding"]
            .as_str()
            .is_some_and(|b| b != binding)
        {
            return Err(fail(409, "account_binding_changed"));
        }
        if !state["accountAccess"].is_object() {
            state["accountAccess"] = json!({"requests":{},"cursor":null});
        }
        if !state["accountAccess"]["requests"].is_object() {
            return Err(fail(503, "invalid_account_state"));
        }
        state["accountAccess"]["binding"] = json!(binding);
        // Readiness is earned from live bot/admin/room checks after each start.
        state["accountAccess"]["rustReady"] = json!(false);
        Ok(())
    }
    fn seal(&self, password: &str, id: &str) -> Result<Value> {
        let key = bytes(&self.password_key)?;
        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|_| fail(503, "invalid_account_secret"))?;
        let iv = rand::random::<[u8; 12]>();
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: password.as_bytes(),
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| fail(503, "password_sealing_failed"))?;
        let (data, tag) = encrypted.split_at(encrypted.len() - 16);
        Ok(json!({"iv":hex(&iv),"data":hex(data),"tag":hex(tag)}))
    }
    fn unseal(&self, row: &Value) -> Result<String> {
        let id = row["id"]
            .as_str()
            .ok_or_else(|| fail(503, "invalid_account_state"))?;
        let read = |key: &str| -> Result<Vec<u8>> {
            bytes(
                row["password"][key]
                    .as_str()
                    .ok_or_else(|| fail(503, "invalid_account_secret"))?,
            )
        };
        let iv = read("iv")?;
        let mut data = read("data")?;
        let tag = read("tag")?;
        if iv.len() != 12 || tag.len() != 16 || data.len() > 1024 {
            return Err(fail(503, "invalid_account_secret"));
        }
        data.extend(tag);
        let key = bytes(&self.password_key)?;
        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|_| fail(503, "invalid_account_secret"))?;
        let clear = cipher
            .decrypt(
                Nonce::from_slice(&iv),
                Payload {
                    msg: &data,
                    aad: id.as_bytes(),
                },
            )
            .map_err(|_| fail(503, "password_unsealing_failed"))?;
        String::from_utf8(clear).map_err(|_| fail(503, "invalid_account_secret"))
    }
}
fn terminal(row: &Value) -> bool {
    matches!(
        row["status"].as_str(),
        Some("registered" | "rejected" | "expired" | "name_unavailable")
    )
}
fn view(row: &Value) -> Value {
    json!({"id":row["id"],"userId":row["userId"],"status":row["status"],"createdAt":row["createdAt"],"expiresAt":row["expiresAt"]})
}
fn finish(state: &mut Value, id: &str, status: &str, now: u64) -> Result<()> {
    let row = &mut state["accountAccess"]["requests"][id];
    let actor = row["decidedBy"]
        .as_str()
        .unwrap_or("account-worker")
        .to_owned();
    row["status"] = json!(status);
    row["finishedAt"] = json!(now);
    row.as_object_mut()
        .ok_or_else(|| fail(503, "invalid_account_state"))?
        .remove("password");
    row.as_object_mut().unwrap().remove("lastError");
    state["audit"].as_array_mut().ok_or_else(||fail(503,"invalid_account_state"))?.push(json!({"atMs":now,"actor":actor,"action":format!("account.{status}"),"target":id,"result":status}));
    Ok(())
}
fn expire(state: &mut Value, now: u64) -> Result<()> {
    let ids: Vec<String> = state["accountAccess"]["requests"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, r)| {
            matches!(
                r["status"].as_str(),
                Some("pending" | "notification_pending")
            ) && r["expiresAt"].as_u64().is_some_and(|n| n <= now)
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        finish(state, &id, "expired", now)?;
    }
    Ok(())
}
fn receipt<'a>(state: &'a Value, id: &str, key: &str) -> Result<&'a Value> {
    let row = &state["accountAccess"]["requests"][id];
    if !hex_id(id, 32)
        || !hex_id(key, 64)
        || row["receiptHash"]
            .as_str()
            .is_none_or(|stored| !bool::from(stored.as_bytes().ct_eq(hash(key).as_bytes())))
    {
        return Err(fail(404, "account_request_missing"));
    }
    Ok(row)
}
fn submit(
    state: &mut Value,
    config: &Configuration,
    input: Value,
    server: &ServerName,
    now: u64,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Input {
        id: String,
        receipt: String,
        username: String,
        password: String,
        display_name: String,
        reason: String,
    }
    let input: Input = serde_json::from_value(input)?;
    if state["accountAccess"]["rustReady"] != true {
        return Err(fail(503, "account_requests_unavailable"));
    }
    if !hex_id(&input.id, 32) || !hex_id(&input.receipt, 64) {
        return Err(fail(400, "invalid_request"));
    }
    let username = &input.username;
    if username.is_empty()
        || username.len() > 64
        || !username.as_bytes()[0].is_ascii_lowercase()
        || !username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.=-".contains(&b))
    {
        return Err(fail(400, "invalid_username"));
    }
    if !(12..=256).contains(&input.password.chars().count()) {
        return Err(fail(400, "invalid_password"));
    }
    let name = input.display_name.trim();
    let reason = input.reason.trim();
    if name.is_empty()
        || name.chars().count() > 128
        || reason.is_empty()
        || reason.chars().count() > 1000
    {
        return Err(fail(400, "invalid_details"));
    }
    let fingerprint = hash(serde_json::to_vec(&json!([username, name, reason]))?);
    if !state["accountAccess"]["requests"][&input.id].is_null() {
        let row = receipt(state, &input.id, &input.receipt)?;
        if row["fingerprint"] != fingerprint
            || !row["password"].is_null() && config.unseal(row)? != input.password
        {
            return Err(fail(409, "request_changed"));
        }
        return Ok(view(row));
    }
    expire(state, now)?;
    let rows = state["accountAccess"]["requests"]
        .as_object()
        .ok_or_else(|| fail(503, "invalid_account_state"))?;
    let user = format!("@{username}:{}", server.as_str());
    if rows.values().any(|r| r["userId"] == user && !terminal(r)) {
        return Err(fail(409, "username_pending"));
    }
    if rows.len() >= 5000 || rows.values().filter(|r| !terminal(r)).count() >= 200 {
        return Err(fail(429, "account_queue_full"));
    }
    let expires = now
        .checked_add(7 * 86400000)
        .ok_or_else(|| fail(400, "invalid_clock"))?;
    let digest = hash(serde_json::to_vec(
        &json!({"id":input.id,"userId":user,"displayName":name,"reason":reason,"expiresAt":expires}),
    )?);
    let row = json!({"id":input.id,"userId":user,"username":username,"displayName":name,"reason":reason,"fingerprint":fingerprint,
        "receiptHash":hash(input.receipt),"password":config.seal(&input.password,&input.id)?,"status":"notification_pending","createdAt":now,"expiresAt":expires,
        "deviceId":format!("PALPO_SIGNUP_{}",hex(&rand::random::<[u8;24]>())),"digest":digest});
    let result = view(&row);
    state["accountAccess"]["requests"][&input.id] = row;
    state["audit"].as_array_mut().ok_or_else(||fail(503,"invalid_account_state"))?.push(json!({"atMs":now,"actor":"applicant","action":"account.request","target":input.id,"result":"pending"}));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_password_ciphertext_and_identity_binding_survive_cutover() {
        let config:Configuration=serde_json::from_value(json!({"botMxid":"@signup:example.test","botToken":"fixture-bot","adminToken":"fixture-admin","approvers":["@admin:example.test"],"passwordKey":"12".repeat(32),"registrationToken":"fixture-registration"})).unwrap();
        // Generated independently with Node 24 createCipheriv/createHash using
        // the production Node sealing and binding formats, not the Rust writer.
        let row = json!({"id":"a".repeat(32),"password":{"iv":"000102030405060708090a0b","data":"3b0a493bfbf5712ceed4842aafb1f7bcf6c12c82ed6a6052","tag":"0ceca4c73474222e7bb033248052beea"}});
        assert_eq!(config.unseal(&row).unwrap(), "Node migration password!");
        let mut wrong = row.clone();
        wrong["id"] = json!("b".repeat(32));
        assert_eq!(
            config.unseal(&wrong).unwrap_err().code,
            "password_unsealing_failed"
        );
        let mut state = json!({"accountAccess":{"binding":"066f474171f6170212773979f646e70087e0132ee2f4a55717792513ce1b4f42","requests":{"original":row},"cursor":"preserved-cursor"}});
        let server = "example.test".to_owned().try_into().unwrap();
        config.bind(&mut state, &server).unwrap();
        assert_eq!(state["accountAccess"]["cursor"], "preserved-cursor");
        assert_eq!(state["accountAccess"]["requests"]["original"], row);
        let mut changed = config.clone();
        changed.password_key = "34".repeat(32);
        assert_eq!(
            changed.bind(&mut state, &server).unwrap_err().code,
            "account_binding_changed"
        );
    }
}
