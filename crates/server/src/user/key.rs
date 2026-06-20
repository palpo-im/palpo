use std::collections::{BTreeMap, HashMap, hash_map};
use std::time::Instant;

use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::json;

use crate::core::client::key::ClaimKeysResBody;
use crate::core::device::DeviceListUpdateContent;
use crate::core::encryption::{CrossSigningKey, DeviceKeys, KeyUsage, OneTimeKey};
use crate::core::federation::key::{
    QueryKeysReqBody, QueryKeysResBody, claim_keys_request, query_keys_request,
};
use crate::core::federation::transaction::{Edu, SigningKeyUpdateContent};
use crate::core::identifiers::*;
use crate::core::serde::{Base64, CanonicalJsonObject, JsonValue};
use crate::core::signatures::{self, PublicKeyMap, PublicKeySet};
use crate::core::{DeviceKeyAlgorithm, UnixMillis, client, federation};
use crate::data::user::{NewDbCrossSignature, NewDbKeyChange};
use crate::exts::*;
use crate::user::clean_signatures;
use crate::{AppError, AppResult, BAD_QUERY_RATE_LIMITER, MatrixError, config, data, sending};

pub async fn query_keys<F: Fn(&UserId) -> bool + Send + Sync>(
    sender_id: Option<&UserId>,
    device_keys_input: &BTreeMap<OwnedUserId, Vec<OwnedDeviceId>>,
    allowed_signatures: F,
    _include_display_names: bool,
) -> AppResult<client::key::KeysResBody> {
    let mut master_keys = BTreeMap::new();
    let mut self_signing_keys = BTreeMap::new();
    let mut user_signing_keys = BTreeMap::new();
    let mut device_keys = BTreeMap::new();
    let mut get_over_federation = HashMap::new();

    for (user_id, device_ids) in device_keys_input {
        if user_id.server_name() != config::get().server_name {
            get_over_federation
                .entry(user_id.server_name())
                .or_insert_with(Vec::new)
                .push((user_id, device_ids));
            continue;
        }

        if device_ids.is_empty() {
            let mut container = BTreeMap::new();
            for device_id in data::user::all_device_ids(user_id).await? {
                if let Some(mut keys) =
                    data::user::get_device_keys_and_sigs(user_id, &device_id).await?
                {
                    let device = data::user::device::get_device(user_id, &device_id).await?;
                    if let Some(display_name) = &device.display_name {
                        keys.unsigned.device_display_name = display_name.to_owned().into();
                    }
                    container.insert(device_id, keys);
                }
            }
            device_keys.insert(user_id.to_owned(), container);
        } else {
            let mut container = BTreeMap::new();
            for device_id in device_ids {
                if let Some(keys) = data::user::get_device_keys_and_sigs(user_id, device_id).await?
                {
                    container.insert(device_id.to_owned(), keys);
                }
            }
            device_keys.insert(user_id.to_owned(), container);
        }

        if let Some(master_key) =
            crate::user::get_allowed_master_key(sender_id, user_id, &allowed_signatures).await?
        {
            master_keys.insert(user_id.to_owned(), master_key);
        }
        if let Some(self_signing_key) =
            crate::user::get_allowed_self_signing_key(sender_id, user_id, &allowed_signatures)
                .await?
        {
            self_signing_keys.insert(user_id.to_owned(), self_signing_key);
        }
        if Some(&**user_id) == sender_id
            && let Some(user_signing_key) = crate::user::get_user_signing_key(user_id).await?
        {
            user_signing_keys.insert(user_id.to_owned(), user_signing_key);
        }
    }

    let mut failures = BTreeMap::new();

    let back_off = |id| match BAD_QUERY_RATE_LIMITER
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .entry(id)
    {
        hash_map::Entry::Vacant(e) => {
            e.insert((Instant::now(), 1));
        }
        hash_map::Entry::Occupied(mut e) => *e.get_mut() = (Instant::now(), e.get().1 + 1),
    };

    let mut futures: FuturesUnordered<_> = get_over_federation
        .into_iter()
        .map(|(server, vec)| async move {
            let mut device_keys_input_fed = BTreeMap::new();
            for (user_id, keys) in vec {
                device_keys_input_fed.insert(user_id.to_owned(), keys.clone());
            }

            let request = query_keys_request(
                &server.origin().await,
                QueryKeysReqBody {
                    device_keys: device_keys_input_fed,
                },
            )?
            .into_inner();

            let response_body = crate::sending::send_federation_request(server, request, None)
                .await?
                .json::<QueryKeysResBody>()
                .await
                .map_err(|_e| AppError::public("Query took too long"));
            Ok::<(&ServerName, AppResult<QueryKeysResBody>), AppError>((server, response_body))
        })
        .collect();

    while let Some(Ok((server, response))) = futures.next().await {
        match response {
            Ok(response) => {
                for (user_id, mut master_key) in response.master_keys {
                    if let Some(our_master_key) = crate::user::get_allowed_master_key(
                        sender_id,
                        &user_id,
                        &allowed_signatures,
                    )
                    .await?
                    {
                        master_key.signatures.extend(our_master_key.signatures);
                    }
                    let json = serde_json::to_value(master_key).expect("to_value always works");
                    let raw =
                        serde_json::from_value(json).expect("RawJson::from_value always works");
                    crate::user::add_cross_signing_keys(
                        &user_id, &raw, &None, &None,
                        false, /* Dont notify. A notification would trigger another key request
                               * resulting in an endless loop */
                    )
                    .await?;
                    master_keys.insert(user_id.to_owned(), raw);
                }

                self_signing_keys.extend(response.self_signing_keys);
                device_keys.extend(response.device_keys);
            }
            _ => {
                back_off(server.to_owned());
                failures.insert(server.to_string(), json!({}));
            }
        }
    }

    Ok(client::key::KeysResBody {
        master_keys,
        self_signing_keys,
        user_signing_keys,
        device_keys,
        failures,
    })
}

pub async fn claim_one_time_keys(
    one_time_keys_input: &BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, DeviceKeyAlgorithm>>,
) -> AppResult<ClaimKeysResBody> {
    let mut one_time_keys = BTreeMap::new();
    let mut get_over_federation = BTreeMap::new();

    for (user_id, map) in one_time_keys_input {
        if user_id.server_name().is_remote() {
            get_over_federation
                .entry(user_id.server_name())
                .or_insert_with(Vec::new)
                .push((user_id, map));
            continue;
        }
        let mut container = BTreeMap::new();
        for (device_id, key_algorithm) in map {
            if let Some(one_time_keys) =
                crate::user::claim_one_time_key(user_id, device_id, key_algorithm).await?
            {
                let mut c = BTreeMap::new();
                c.insert(one_time_keys.0, one_time_keys.1);
                container.insert(device_id.clone(), c);
            }
        }
        if !container.is_empty() {
            one_time_keys.insert(user_id.clone(), container);
        }
    }

    let mut failures = BTreeMap::new();

    let mut futures: FuturesUnordered<_> = FuturesUnordered::new();
    for (server, vec) in get_over_federation.into_iter() {
        let mut one_time_keys_input_fed = BTreeMap::new();
        for (user_id, keys) in vec {
            one_time_keys_input_fed.insert(user_id.clone(), keys.clone());
        }
        let request = claim_keys_request(
            &server.origin().await,
            federation::key::ClaimKeysReqBody {
                timeout: None,
                one_time_keys: one_time_keys_input_fed,
            },
        )?
        .into_inner();
        futures.push(async move {
            (
                server,
                crate::sending::send_federation_request(server, request, None).await,
            )
        });
    }
    while let Some((server, response)) = futures.next().await {
        match response {
            Ok(response) => match response.json::<federation::key::ClaimKeysResBody>().await {
                Ok(keys) => {
                    one_time_keys.extend(keys.one_time_keys);
                }
                Err(_e) => {
                    failures.insert(server.to_string(), json!({}));
                }
            },
            Err(_e) => {
                failures.insert(server.to_string(), json!({}));
            }
        }
    }
    Ok(ClaimKeysResBody {
        failures,
        one_time_keys,
    })
}

pub async fn get_master_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    let key_data = data::user::key::get_cross_signing_key(user_id, "master").await?;
    if let Some(key_data) = key_data {
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub async fn get_allowed_master_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &(dyn Fn(&UserId) -> bool + Send + Sync),
) -> AppResult<Option<CrossSigningKey>> {
    let key_data = data::user::key::get_cross_signing_key_and_sigs(user_id, "master").await?;
    if let Some(mut key_data) = key_data {
        clean_signatures(&mut key_data, sender_id, user_id, allowed_signatures)?;
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub async fn get_self_signing_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    let key_data = data::user::key::get_cross_signing_key(user_id, "self_signing").await?;
    if let Some(key_data) = key_data {
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}
pub async fn get_allowed_self_signing_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &(dyn Fn(&UserId) -> bool + Send + Sync),
) -> AppResult<Option<CrossSigningKey>> {
    let key_data = data::user::key::get_cross_signing_key_and_sigs(user_id, "self_signing").await?;
    if let Some(mut key_data) = key_data {
        clean_signatures(&mut key_data, sender_id, user_id, allowed_signatures)?;
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub async fn get_user_signing_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    let key_data = data::user::key::get_cross_signing_key(user_id, "user_signing").await?;
    Ok(key_data.and_then(|data| serde_json::from_value(data).ok()))
}

pub async fn add_one_time_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_id: &DeviceKeyId,
    one_time_key: &OneTimeKey,
) -> AppResult<()> {
    data::user::key::add_one_time_key(user_id, device_id, key_id, one_time_key).await?;
    Ok(())
}

pub async fn claim_one_time_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_algorithm: &DeviceKeyAlgorithm,
) -> AppResult<Option<(OwnedDeviceKeyId, OneTimeKey)>> {
    Ok(data::user::key::claim_one_time_key(user_id, device_id, key_algorithm).await?)
}

pub async fn add_device_keys(
    user_id: &UserId,
    device_id: &DeviceId,
    device_keys: &DeviceKeys,
) -> AppResult<()> {
    data::user::add_device_keys(user_id, device_id, device_keys).await?;
    mark_device_key_update(user_id, device_id).await?;
    send_device_key_update(user_id, device_id).await?;
    Ok(())
}

pub async fn add_cross_signing_keys(
    user_id: &UserId,
    master_key: &CrossSigningKey,
    self_signing_key: &Option<CrossSigningKey>,
    user_signing_key: &Option<CrossSigningKey>,
    notify: bool,
) -> AppResult<()> {
    add_cross_signing_key_updates(
        user_id,
        Some(master_key),
        self_signing_key.as_ref(),
        user_signing_key.as_ref(),
        notify,
    )
    .await
}

pub async fn add_cross_signing_key_updates(
    user_id: &UserId,
    master_key: Option<&CrossSigningKey>,
    self_signing_key: Option<&CrossSigningKey>,
    user_signing_key: Option<&CrossSigningKey>,
    notify: bool,
) -> AppResult<()> {
    let existing_master_key =
        if master_key.is_none() && (self_signing_key.is_some() || user_signing_key.is_some()) {
            get_master_key(user_id).await?
        } else {
            None
        };

    let master_key_for_signatures = master_key.or(existing_master_key.as_ref());

    if let Some(master_key) = master_key {
        validate_cross_signing_key(user_id, CrossSigningKeyKind::Master, master_key)?;
    }

    if let Some(self_signing_key) = self_signing_key {
        validate_cross_signing_key(user_id, CrossSigningKeyKind::SelfSigning, self_signing_key)?;
        verify_cross_signing_key_signature(
            self_signing_key,
            user_id,
            master_key_for_signatures
                .ok_or(MatrixError::invalid_param("Missing master signing key."))?,
        )?;
    }

    if let Some(user_signing_key) = user_signing_key {
        validate_cross_signing_key(user_id, CrossSigningKeyKind::UserSigning, user_signing_key)?;
        verify_cross_signing_key_signature(
            user_signing_key,
            user_id,
            master_key_for_signatures
                .ok_or(MatrixError::invalid_param("Missing master signing key."))?,
        )?;
    }

    if let Some(master_key) = master_key {
        add_cross_signing_key(user_id, "master", master_key).await?;
    }

    if let Some(self_signing_key) = self_signing_key {
        add_cross_signing_key(user_id, "self_signing", self_signing_key).await?;
    }

    if let Some(user_signing_key) = user_signing_key {
        add_cross_signing_key(user_id, "user_signing", user_signing_key).await?;
    }

    if notify {
        mark_signing_key_update(user_id).await?;
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum CrossSigningKeyKind {
    Master,
    SelfSigning,
    UserSigning,
}

impl CrossSigningKeyKind {
    fn matches_usage(self, usage: &KeyUsage) -> bool {
        matches!(
            (self, usage),
            (Self::Master, KeyUsage::Master)
                | (Self::SelfSigning, KeyUsage::SelfSigning)
                | (Self::UserSigning, KeyUsage::UserSigning)
        )
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Master => "Master",
            Self::SelfSigning => "Self signing",
            Self::UserSigning => "User signing",
        }
    }
}

fn validate_cross_signing_key(
    user_id: &UserId,
    kind: CrossSigningKeyKind,
    key: &CrossSigningKey,
) -> AppResult<()> {
    if &key.user_id != user_id {
        return Err(MatrixError::invalid_param(format!(
            "{} key user_id does not match the authenticated user.",
            kind.display_name()
        ))
        .into());
    }

    if key.usage.len() != 1 || !key.usage.iter().any(|usage| kind.matches_usage(usage)) {
        return Err(MatrixError::invalid_param(format!(
            "{} key contained invalid usage.",
            kind.display_name()
        ))
        .into());
    }

    if key.keys.len() != 1 {
        return Err(MatrixError::invalid_param(format!(
            "{} key must contain exactly one key.",
            kind.display_name()
        ))
        .into());
    }

    let (key_id, public_key) = key
        .keys
        .iter()
        .next()
        .expect("cross-signing key length checked above");
    if key_id.algorithm() != DeviceKeyAlgorithm::Ed25519 {
        return Err(MatrixError::invalid_param(format!(
            "{} key must use ed25519.",
            kind.display_name()
        ))
        .into());
    }
    let _: Base64 = Base64::parse(public_key).map_err(|_| {
        MatrixError::invalid_param(format!(
            "{} key contained an invalid public key.",
            kind.display_name()
        ))
    })?;

    Ok(())
}

fn verify_cross_signing_key_signature(
    signed_key: &CrossSigningKey,
    signer_user_id: &UserId,
    signer_key: &CrossSigningKey,
) -> AppResult<()> {
    let signer_signatures =
        signed_key
            .signatures
            .get(signer_user_id)
            .ok_or(MatrixError::invalid_param(
                "Missing cross-signing key signature.",
            ))?;
    let mut public_keys = PublicKeySet::new();
    for (key_id, public_key) in &signer_key.keys {
        if signer_signatures.contains_key(key_id) {
            public_keys.insert(
                key_id.to_string(),
                Base64::parse(public_key).map_err(|_| {
                    MatrixError::invalid_param("Master key contained an invalid public key.")
                })?,
            );
        }
    }
    if public_keys.is_empty() {
        return Err(
            MatrixError::invalid_param("Missing signature from the user's master key.").into(),
        );
    }

    let mut value = serde_json::to_value(signed_key)?;
    let Some(object) = value.as_object_mut() else {
        return Err(MatrixError::invalid_param("Invalid cross-signing key.").into());
    };
    let mut signatures = serde_json::Map::new();
    signatures.insert(
        signer_user_id.to_string(),
        JsonValue::Object(
            serde_json::to_value(signer_signatures)?
                .as_object()
                .cloned()
                .ok_or(MatrixError::invalid_param(
                    "Invalid cross-signing key signatures.",
                ))?,
        ),
    );
    object.insert("signatures".to_owned(), JsonValue::Object(signatures));
    let object: CanonicalJsonObject = serde_json::from_value(value)
        .map_err(|_| MatrixError::invalid_param("Invalid cross-signing key canonical JSON."))?;

    let mut public_key_map = PublicKeyMap::new();
    public_key_map.insert(signer_user_id.to_string(), public_keys);
    signatures::verify_json(&public_key_map, &object)
        .map_err(|_| MatrixError::invalid_param("Invalid cross-signing key signature."))?;

    Ok(())
}

async fn add_cross_signing_key(
    user_id: &UserId,
    key_type: &str,
    key: &CrossSigningKey,
) -> AppResult<()> {
    data::user::key::add_cross_signing_key(user_id, key_type, key).await?;
    Ok(())
}

pub async fn sign_key(
    target_user_id: &UserId,
    target_device_id: &str,
    signature: (String, String),
    sender_id: &UserId,
) -> AppResult<()> {
    // let cross_signing_key = e2e_cross_signing_keys::table
    //     .filter(e2e_cross_signing_keys::user_id.eq(target_id))
    //     .filter(e2e_cross_signing_keys::key_type.eq("master"))
    //     .order_by(e2e_cross_signing_keys::id.desc())
    //     .first::<DbCrossSigningKey>(&mut connect()?)?;
    // let mut cross_signing_key: CrossSigningKey =
    // serde_json::from_value(cross_signing_key.key_data.clone())?;
    let origin_key_id = DeviceKeyId::parse(&signature.0)?.to_owned();

    // cross_signing_key
    //     .signatures
    //     .entry(sender_id.to_owned())
    //     .or_defaut()
    //     .insert(key_id.clone(), signature.1);

    data::user::key::add_cross_signing_sig(NewDbCrossSignature {
        origin_user_id: sender_id.to_owned(),
        origin_key_id,
        target_user_id: target_user_id.to_owned(),
        target_device_id: OwnedDeviceId::from(target_device_id),
        signature: signature.1,
    })
    .await?;
    mark_signing_key_update(target_user_id).await
}

pub async fn mark_signing_key_update(user_id: &UserId) -> AppResult<()> {
    let changed_at = UnixMillis::now();

    let joined_rooms = data::user::joined_rooms(user_id).await?;
    for room_id in &joined_rooms {
        // // Don't send key updates to unencrypted rooms
        // if state::get_state(&room_id, &StateEventType::RoomEncryption, "")?.is_none() {
        //     continue;
        // }

        let change = NewDbKeyChange {
            user_id: user_id.to_owned(),
            room_id: Some(room_id.to_owned()),
            changed_at,
            occur_sn: data::next_sn().await?,
        };

        data::user::key::replace_key_change(&change).await?;
    }

    let change = NewDbKeyChange {
        user_id: user_id.to_owned(),
        room_id: None,
        changed_at,
        occur_sn: data::next_sn().await?,
    };

    data::user::key::replace_key_change(&change).await?;

    if user_id.is_local() {
        let remote_servers = data::room::joined_servers_for_rooms(&joined_rooms).await?;

        let content = SigningKeyUpdateContent::new(user_id.to_owned());
        let edu = Edu::SigningKeyUpdate(content);

        let _ = sending::send_edu_servers(remote_servers.into_iter(), &edu).await;
    }

    Ok(())
}

pub async fn mark_device_key_update(user_id: &UserId, _device_id: &DeviceId) -> AppResult<()> {
    let changed_at = UnixMillis::now();
    let joined_rooms = data::user::joined_rooms(user_id).await?;
    let occur_sn = data::next_sn().await?;
    for room_id in &joined_rooms {
        // comment for testing
        // // Don't send key updates to unencrypted rooms
        // if state::get_state(&room_id, &StateEventType::RoomEncryption, "")?.is_none() {
        //     continue;
        // }

        let change = NewDbKeyChange {
            user_id: user_id.to_owned(),
            room_id: Some(room_id.to_owned()),
            changed_at,
            occur_sn,
        };

        data::user::key::replace_key_change(&change).await?;
    }

    let change = NewDbKeyChange {
        user_id: user_id.to_owned(),
        room_id: None,
        changed_at,
        occur_sn,
    };

    data::user::key::replace_key_change(&change).await?;

    Ok(())
}

pub async fn mark_device_key_update_with_joined_rooms(
    user_id: &UserId,
    _device_id: &DeviceId,
    joined_rooms: &[OwnedRoomId],
) -> AppResult<()> {
    let changed_at = UnixMillis::now();
    let occur_sn = data::next_sn().await?;
    for room_id in joined_rooms {
        let change = NewDbKeyChange {
            user_id: user_id.to_owned(),
            room_id: Some(room_id.to_owned()),
            changed_at,
            occur_sn,
        };

        data::user::key::replace_key_change(&change).await?;
    }
    Ok(())
}

pub async fn send_device_key_update(user_id: &UserId, device_id: &DeviceId) -> AppResult<()> {
    let joined_rooms = data::user::joined_rooms(user_id).await?;
    send_device_key_update_with_joined_rooms(user_id, device_id, &joined_rooms).await
}

async fn send_device_key_update_with_joined_rooms(
    user_id: &UserId,
    device_id: &DeviceId,
    joined_rooms: &[OwnedRoomId],
) -> AppResult<()> {
    if user_id.is_remote() {
        return Ok(());
    }
    let remote_servers = data::room::joined_servers_for_rooms(joined_rooms).await?;

    let content = DeviceListUpdateContent::new(
        user_id.to_owned(),
        device_id.to_owned(),
        data::next_sn().await? as u64,
    );
    let edu = Edu::DeviceListUpdate(content);

    sending::send_edu_servers(remote_servers.into_iter(), &edu).await
}
