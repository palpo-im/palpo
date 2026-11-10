//! Verify key documents before projecting them into the legacy cache model.
use sha2::{Digest, Sha256};

use super::*;

fn digest(raw: &serde_json::Value) -> AppResult<String> {
    let object: CanonicalJsonObject = serde_json::from_value(raw.clone())?;
    Ok(Sha256::digest(serde_json::to_vec(&object)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) async fn persist(
    raw: &serde_json::Value,
    keys: &ServerSigningKeys,
    source: &str,
    source_server: &ServerName,
) -> AppResult<()> {
    data::misc::store_verified_key_document(
        &keys.server_name,
        raw,
        &digest(raw)?,
        source,
        source_server,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
pub(super) static RAW_DOCUMENTS_LOADED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn notaries() -> Vec<String> {
    config::get()
        .trusted_servers
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn checked_records(
    server: &ServerName,
    records: Vec<data::misc::VerifiedKeyDocument>,
) -> AppResult<Vec<(ServerSigningKeys, UnixMillis, serde_json::Value)>> {
    #[cfg(test)]
    RAW_DOCUMENTS_LOADED.fetch_add(records.len(), std::sync::atomic::Ordering::Relaxed);
    let mut accepted = Vec::new();
    for record in records {
        let trusted_source = match record.source.as_str() {
            "direct" => record.source_server == server.as_str(),
            "notary" => config::get()
                .trusted_servers
                .iter()
                .any(|notary| notary.as_str() == record.source_server),
            _ => false,
        };
        if !trusted_source
            || record.validation_version != 1
            || record.validated_at < 0
            || digest(&record.key_data)? != record.document_sha256
        {
            continue;
        }
        if let Ok(keys) = origin(&record.key_data, server) {
            accepted.push((
                keys,
                UnixMillis(record.validated_at as u64),
                record.key_data,
            ));
        }
    }
    Ok(accepted)
}

async fn cached_documents(
    server: &ServerName,
) -> AppResult<Vec<(ServerSigningKeys, UnixMillis, serde_json::Value)>> {
    checked_records(
        server,
        data::misc::latest_verified_key_document(server, &notaries()).await?,
    )
}

pub(super) async fn event_documents(
    server: &ServerName,
    id: &ServerSigningKeyId,
    timestamp: UnixMillis,
    enforce: bool,
) -> AppResult<Vec<(ServerSigningKeys, UnixMillis)>> {
    let timestamp = timestamp.0.min(i64::MAX as u64) as i64;
    Ok(checked_records(
        server,
        data::misc::verified_event_key_documents(
            server,
            id.as_str(),
            timestamp,
            enforce,
            &notaries(),
        )
        .await?,
    )?
    .into_iter()
    .map(|(keys, at, _)| (keys, at))
    .collect())
}

pub(super) async fn retirement(
    server: &ServerName,
    id: &ServerSigningKeyId,
) -> AppResult<Option<UnixMillis>> {
    Ok(
        data::misc::verified_key_retirement(server, id.as_str(), &notaries())
            .await?
            .map(|value| UnixMillis(value.max(0) as u64)),
    )
}

pub(super) async fn cached(server: &ServerName) -> AppResult<Vec<(ServerSigningKeys, UnixMillis)>> {
    Ok(cached_documents(server)
        .await?
        .into_iter()
        .map(|(keys, at, _)| (keys, at))
        .collect())
}

pub(super) async fn cached_raw(server: &ServerName) -> AppResult<Option<serde_json::Value>> {
    Ok(cached_documents(server)
        .await?
        .into_iter()
        .next()
        .map(|(_, _, raw)| raw))
}

pub(super) async fn request_key(
    server: &ServerName,
    id: &ServerSigningKeyId,
) -> AppResult<Option<VerifyKey>> {
    if retirement(server, id).await?.is_some() {
        return Ok(None);
    }
    latest_request_key(server, id).await
}

pub(super) async fn latest_request_key(
    server: &ServerName,
    id: &ServerSigningKeyId,
) -> AppResult<Option<VerifyKey>> {
    let documents = cached(server).await?;
    let Some((mut latest, fetched)) = documents.into_iter().next() else {
        return Ok(None);
    };
    validity::bound_cached_keys(&mut latest, fetched);
    if latest.valid_until_ts < UnixMillis::now() {
        return Ok(None);
    }
    Ok(extract_key(latest, id))
}

/// Test fixtures explicitly establish a trusted source and produce a real
/// origin signature; production never promotes legacy merged cache entries.
#[cfg(test)]
pub(crate) async fn seed(mut keys: ServerSigningKeys, pair: &signatures::Ed25519KeyPair) {
    keys.verify_keys.insert(
        format!("ed25519:{}", pair.version()).try_into().unwrap(),
        VerifyKey::from_bytes(pair.public_key().to_vec()),
    );
    keys.signatures.clear();
    let mut raw: CanonicalJsonObject =
        serde_json::from_value(serde_json::to_value(&keys).unwrap()).unwrap();
    signatures::sign_json(keys.server_name.as_str(), pair, &mut raw).unwrap();
    let raw = serde_json::to_value(raw).unwrap();
    let checked = origin(&raw, &keys.server_name).unwrap();
    persist(&raw, &checked, "direct", &keys.server_name)
        .await
        .unwrap();
    add_signing_keys(checked).await.unwrap();
}

pub(super) fn signature(
    raw: &serde_json::Value,
    signer: &ServerName,
    keys: &VerifyKeys,
) -> AppResult<()> {
    let mut object: CanonicalJsonObject = serde_json::from_value(raw.clone())?;
    let signature = object
        .get("signatures")
        .and_then(|value| value.as_object())
        .and_then(|signatures| signatures.get(signer.as_str()))
        .cloned()
        .ok_or_else(|| AppError::public("key document lacks required signature"))?;
    // Additional signatures are independent attestations; verify precisely the
    // required signer while preserving every field covered by its signature.
    object.insert(
        "signatures".into(),
        crate::core::serde::CanonicalJsonValue::Object(BTreeMap::from([(
            signer.to_string(),
            signature,
        )])),
    );
    let public_keys = BTreeMap::from([(
        signer.to_string(),
        keys.iter()
            .map(|(id, key)| (id.to_string(), key.key.clone()))
            .collect(),
    )]);
    signatures::verify_json(&public_keys, &object)?;
    Ok(())
}

pub(super) fn current_keys(keys: &ServerSigningKeys) -> VerifyKeys {
    keys.verify_keys
        .iter()
        .filter(|(id, _)| !keys.old_verify_keys.contains_key(*id))
        .map(|(id, key)| (id.clone(), key.clone()))
        .collect()
}

pub(super) fn origin(
    raw: &serde_json::Value,
    expected: &ServerName,
) -> AppResult<ServerSigningKeys> {
    let keys: ServerSigningKeys = serde_json::from_value(raw.clone())?;
    if keys.server_name != expected {
        return Err(AppError::public("key document has unexpected server_name"));
    }
    signature(raw, expected, &current_keys(&keys))?;
    Ok(keys)
}

pub(super) fn relevant(
    keys: &ServerSigningKeys,
    requested: Option<&std::collections::BTreeSet<crate::core::OwnedServerSigningKeyId>>,
) -> bool {
    keys.verify_keys
        .keys()
        .chain(keys.old_verify_keys.keys())
        .any(|id| requested.is_none_or(|ids| ids.contains(id)))
}

pub(super) async fn notary_documents(
    notary: &ServerName,
    response: serde_json::Value,
    requested: &BTreeMap<
        crate::core::OwnedServerName,
        Option<std::collections::BTreeSet<crate::core::OwnedServerSigningKeyId>>,
    >,
) -> AppResult<Vec<ServerSigningKeys>> {
    let documents = response
        .get("server_keys")
        .and_then(|value| value.as_array())
        .ok_or_else(|| AppError::public("invalid notary response"))?;
    let scoped: Vec<_> = documents
        .iter()
        .filter(|raw| {
            raw.get("server_name")
                .and_then(|value| value.as_str())
                .is_some_and(|name| {
                    requested.iter().any(|(server, ids)| {
                        server.as_str() == name
                            && serde_json::from_value::<ServerSigningKeys>((*raw).clone())
                                .is_ok_and(|keys| relevant(&keys, ids.as_ref()))
                    })
                })
        })
        .collect();
    if scoped.is_empty() {
        return Ok(Vec::new());
    }
    // The configured HTTPS endpoint authenticates this bootstrap. The key
    // document's self-signature alone does not establish a notary's identity.
    // Never bootstrap through notaries or reuse the unproven generic cache.
    let (_, mut bootstrap) = super::request::fetch_server_document(notary).await?;
    let now = UnixMillis::now();
    validity::bound_cached_keys(&mut bootstrap, now);
    if bootstrap.valid_until_ts < now {
        return Err(AppError::public("notary bootstrap key document is expired"));
    }
    let notary_keys = current_keys(&bootstrap);
    let mut accepted = Vec::new();
    for raw in scoped {
        let Some(server) = raw
            .get("server_name")
            .and_then(|value| value.as_str())
            .and_then(|name| <&ServerName>::try_from(name).ok())
        else {
            continue;
        };
        match origin(raw, server).and_then(|keys| {
            signature(raw, notary, &notary_keys)?;
            Ok(keys)
        }) {
            Ok(keys) => {
                persist(raw, &keys, "notary", notary).await?;
                accepted.push(keys);
            }
            Err(_) => warn!(%notary,%server,"rejecting unverified notary key document"),
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_document_verification_requires_the_requested_signer_and_preserves_extensions() {
        let pair = signatures::Ed25519KeyPair::from_der(
            &signatures::Ed25519KeyPair::generate().unwrap(),
            "unit".into(),
        )
        .unwrap();
        let server: &ServerName = "document.example".try_into().unwrap();
        let mut keys = ServerSigningKeys::new(server.to_owned(), UnixMillis(123));
        keys.verify_keys.insert(
            "ed25519:unit".try_into().unwrap(),
            VerifyKey::from_bytes(pair.public_key().to_vec()),
        );
        let mut unsigned = serde_json::to_value(&keys).unwrap();
        unsigned["org.example.extension"] = serde_json::json!({"covered":true});
        assert!(origin(&unsigned, server).is_err());
        let mut canonical: CanonicalJsonObject = serde_json::from_value(unsigned).unwrap();
        signatures::sign_json(server.as_str(), &pair, &mut canonical).unwrap();
        let signed = serde_json::to_value(canonical).unwrap();
        let parsed = origin(&signed, server).unwrap();
        assert_eq!(parsed.server_name, server);
        assert!(origin(&signed, "wrong.example".try_into().unwrap()).is_err());
        let mut tampered = signed.clone();
        tampered["org.example.extension"] = serde_json::json!({"covered":false});
        assert!(origin(&tampered, server).is_err());
        let mut retired = keys;
        let current = retired.verify_keys["ed25519:unit"].clone();
        retired.old_verify_keys.insert(
            "ed25519:unit".try_into().unwrap(),
            crate::core::federation::discovery::OldVerifyKey::new(UnixMillis(100), current.key),
        );
        assert!(extract_key(retired, "ed25519:unit".try_into().unwrap()).is_none());
    }
}
