use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::key::{Failure, UploadSignaturesReqBody, UploadSignaturesResBody};
use crate::core::encryption::{CrossSigningKey, DeviceKeys, KeyUsage};
use crate::core::identifiers::*;
use crate::core::serde::{Base64, CanonicalJsonObject, JsonValue, RawJsonValue};
use crate::core::signatures::{self, PublicKeyMap, PublicKeySet};
use crate::{AppResult, AuthArgs, DepotExt, JsonResult, data, json_ok};

/// #POST /_matrix/client/r0/keys/signatures/upload
/// Uploads end-to-end key signatures from the sender user.
#[endpoint]
pub(super) async fn upload(
    _aa: AuthArgs,
    body: JsonBody<UploadSignaturesReqBody>,
    depot: &mut Depot,
) -> JsonResult<UploadSignaturesResBody> {
    let authed = depot.authed_info()?;
    let body = body.into_inner();
    let mut failures: BTreeMap<OwnedUserId, BTreeMap<String, Failure>> = BTreeMap::new();

    for (user_id, keys) in &body.0 {
        for (key_id, key) in keys.iter() {
            if let Some(failure) =
                process_signed_key(authed.user_id(), user_id, key_id, key).await?
            {
                failures
                    .entry(user_id.to_owned())
                    .or_default()
                    .insert(key_id.to_owned(), failure);
            }
        }
    }

    json_ok(UploadSignaturesResBody::new(failures))
}

async fn process_signed_key(
    sender_id: &UserId,
    target_user_id: &UserId,
    target_key_id: &str,
    key: &RawJsonValue,
) -> AppResult<Option<Failure>> {
    let key_value = match serde_json::from_str::<JsonValue>(key.get()) {
        Ok(value) => value,
        Err(_) => return Ok(Some(Failure::invalid_signature("Invalid signed key JSON."))),
    };

    if let Some(failure) = validate_target_key(target_user_id, target_key_id, &key_value).await? {
        return Ok(Some(failure));
    }

    let sender_signatures = match sender_signature_map(sender_id, &key_value) {
        Ok(signatures) => signatures,
        Err(failure) => return Ok(Some(failure)),
    };

    if sender_signatures.values().any(|value| !value.is_string()) {
        return Ok(Some(Failure::invalid_signature(
            "Signature values must be strings.",
        )));
    }

    let signing_key_ids = sender_signatures.keys().cloned().collect::<Vec<_>>();
    let public_keys = signing_public_keys(sender_id, &signing_key_ids).await?;
    if public_keys.is_empty() {
        return Ok(Some(Failure::invalid_signature(
            "No known signing key for authenticated user.",
        )));
    }

    let verification_object =
        match canonical_object_for_sender_signature(sender_id, &key_value, sender_signatures) {
            Ok(object) => object,
            Err(failure) => return Ok(Some(failure)),
        };
    let mut public_key_map = PublicKeyMap::new();
    public_key_map.insert(sender_id.to_string(), public_keys.clone());
    if signatures::verify_json(&public_key_map, &verification_object).is_err() {
        return Ok(Some(Failure::invalid_signature(
            "Signature does not verify against the authenticated user's keys.",
        )));
    }

    for (signing_key_id, signature) in sender_signatures {
        if public_keys.contains_key(signing_key_id)
            && let Some(signature) = signature.as_str()
        {
            crate::user::sign_key(
                target_user_id,
                target_key_id,
                (signing_key_id.to_owned(), signature.to_owned()),
                sender_id,
            )
            .await?;
        }
    }

    Ok(None)
}

async fn validate_target_key(
    target_user_id: &UserId,
    target_key_id: &str,
    key_value: &JsonValue,
) -> AppResult<Option<Failure>> {
    if let Ok(device_keys) = serde_json::from_value::<DeviceKeys>(key_value.clone()) {
        if &device_keys.user_id != target_user_id || device_keys.device_id.as_str() != target_key_id
        {
            return Ok(Some(Failure::invalid_signature(
                "Signed device key does not match the target key.",
            )));
        }

        if data::user::key::get_device_keys(target_user_id, &device_keys.device_id)
            .await?
            .is_none()
        {
            return Ok(Some(Failure::invalid_signature(
                "Unknown target device key.",
            )));
        }

        return Ok(None);
    }

    if let Ok(cross_signing_key) = serde_json::from_value::<CrossSigningKey>(key_value.clone()) {
        if &cross_signing_key.user_id != target_user_id
            || !cross_signing_key
                .keys
                .keys()
                .any(|key_id| key_id.as_str() == target_key_id)
        {
            return Ok(Some(Failure::invalid_signature(
                "Signed cross-signing key does not match the target key.",
            )));
        }

        let Some(key_type) = cross_signing_key_type(&cross_signing_key) else {
            return Ok(Some(Failure::invalid_signature(
                "Unknown cross-signing key usage.",
            )));
        };

        let Some(existing_key) =
            data::user::key::get_cross_signing_key(target_user_id, key_type).await?
        else {
            return Ok(Some(Failure::invalid_signature(
                "Unknown target cross-signing key.",
            )));
        };
        let Ok(existing_key) = serde_json::from_value::<CrossSigningKey>(existing_key) else {
            return Ok(Some(Failure::invalid_signature(
                "Stored target cross-signing key is invalid.",
            )));
        };

        if !existing_key
            .keys
            .keys()
            .any(|key_id| key_id.as_str() == target_key_id)
        {
            return Ok(Some(Failure::invalid_signature(
                "Target cross-signing key does not match the stored key.",
            )));
        }

        return Ok(None);
    }

    Ok(Some(Failure::invalid_signature(
        "Signed key is neither a device key nor a cross-signing key.",
    )))
}

fn sender_signature_map<'a>(
    sender_id: &UserId,
    key_value: &'a JsonValue,
) -> Result<&'a serde_json::Map<String, JsonValue>, Failure> {
    key_value
        .get("signatures")
        .and_then(JsonValue::as_object)
        .and_then(|signatures| signatures.get(sender_id.as_str()))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| Failure::invalid_signature("Missing signature from authenticated user."))
}

fn canonical_object_for_sender_signature(
    sender_id: &UserId,
    key_value: &JsonValue,
    sender_signatures: &serde_json::Map<String, JsonValue>,
) -> Result<CanonicalJsonObject, Failure> {
    let mut value = key_value.clone();
    let Some(object) = value.as_object_mut() else {
        return Err(Failure::invalid_signature(
            "Signed key must be a JSON object.",
        ));
    };

    let mut signatures = serde_json::Map::new();
    signatures.insert(
        sender_id.to_string(),
        JsonValue::Object(sender_signatures.clone()),
    );
    object.insert("signatures".to_owned(), JsonValue::Object(signatures));

    serde_json::from_value(value)
        .map_err(|_| Failure::invalid_signature("Signed key is not canonical JSON."))
}

async fn signing_public_keys(
    sender_id: &UserId,
    signing_key_ids: &[String],
) -> AppResult<PublicKeySet> {
    let mut public_keys = PublicKeySet::new();

    add_cross_signing_public_keys(
        &mut public_keys,
        crate::user::get_master_key(sender_id).await?.as_ref(),
        signing_key_ids,
    );
    add_cross_signing_public_keys(
        &mut public_keys,
        crate::user::get_self_signing_key(sender_id).await?.as_ref(),
        signing_key_ids,
    );
    add_cross_signing_public_keys(
        &mut public_keys,
        crate::user::get_user_signing_key(sender_id).await?.as_ref(),
        signing_key_ids,
    );

    for signing_key_id in signing_key_ids {
        if public_keys.contains_key(signing_key_id) {
            continue;
        }

        let Ok(device_key_id) = DeviceKeyId::parse(signing_key_id) else {
            continue;
        };
        let Some(device_keys) =
            data::user::key::get_device_keys(sender_id, device_key_id.key_name()).await?
        else {
            continue;
        };
        if let Some(public_key) = device_keys.keys.get(&device_key_id) {
            add_public_key(&mut public_keys, signing_key_id, public_key);
        }
    }

    Ok(public_keys)
}

fn add_cross_signing_public_keys(
    public_keys: &mut PublicKeySet,
    key: Option<&CrossSigningKey>,
    signing_key_ids: &[String],
) {
    let Some(key) = key else {
        return;
    };

    for signing_key_id in signing_key_ids {
        if let Some(public_key) = key.keys.get(signing_key_id.as_str()) {
            add_public_key(public_keys, signing_key_id, public_key);
        }
    }
}

fn add_public_key(public_keys: &mut PublicKeySet, key_id: &str, public_key: &str) {
    if let Ok(public_key) = Base64::parse(public_key) {
        public_keys.insert(key_id.to_owned(), public_key);
    }
}

fn cross_signing_key_type(key: &CrossSigningKey) -> Option<&'static str> {
    if key
        .usage
        .iter()
        .any(|usage| matches!(usage, KeyUsage::Master))
    {
        Some("master")
    } else if key
        .usage
        .iter()
        .any(|usage| matches!(usage, KeyUsage::SelfSigning))
    {
        Some("self_signing")
    } else if key
        .usage
        .iter()
        .any(|usage| matches!(usage, KeyUsage::UserSigning))
    {
        Some("user_signing")
    } else {
        None
    }
}
