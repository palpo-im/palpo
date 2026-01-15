use std::collections::{BTreeMap, HashMap, hash_map};
use std::time::Instant;

use diesel::prelude::*;
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::json;

use crate::core::client::key::ClaimKeysResBody;
use crate::core::device::DeviceListUpdateContent;
use crate::core::encryption::{CrossSigningKey, DeviceKeys, OneTimeKey};
use crate::core::federation::key::{
    QueryKeysReqBody, QueryKeysResBody, claim_keys_request, query_keys_request,
};
use crate::core::federation::transaction::{Edu, SigningKeyUpdateContent};
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::{DeviceKeyAlgorithm, UnixMillis, client, federation};
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::{
    DbOneTimeKey, NewDbCrossSignature, NewDbCrossSigningKey, NewDbKeyChange, NewDbOneTimeKey,
};
use crate::exts::*;
use crate::user::clean_signatures;
use crate::{AppError, AppResult, BAD_QUERY_RATE_LIMITER, MatrixError, config, data, sending};

pub async fn query_keys<F: Fn(&UserId) -> bool>(
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
            for device_id in data::user::all_device_ids(user_id)? {
                if let Some(mut keys) = data::user::get_device_keys_and_sigs(user_id, &device_id)? {
                    let device = data::user::device::get_device(user_id, &device_id)?;
                    if let Some(display_name) = &device.display_name {
                        keys.unsigned.device_display_name = display_name.to_owned().into();
                    }
                    container.insert(device_id, keys);
                }
            }
            device_keys.insert(user_id.to_owned(), container);
        } else {
            for device_id in device_ids {
                let mut container = BTreeMap::new();
                if let Some(keys) = data::user::get_device_keys_and_sigs(user_id, device_id)? {
                    container.insert(device_id.to_owned(), keys);
                }
                device_keys.insert(user_id.to_owned(), container);
            }
        }

        if let Some(master_key) =
            crate::user::get_allowed_master_key(sender_id, user_id, &allowed_signatures)?
        {
            master_keys.insert(user_id.to_owned(), master_key);
        }
        if let Some(self_signing_key) =
            crate::user::get_allowed_self_signing_key(sender_id, user_id, &allowed_signatures)?
        {
            self_signing_keys.insert(user_id.to_owned(), self_signing_key);
        }
        if Some(&**user_id) == sender_id
            && let Some(user_signing_key) = crate::user::get_user_signing_key(user_id)?
        {
            user_signing_keys.insert(user_id.to_owned(), user_signing_key);
        }
    }

    let mut failures = BTreeMap::new();

    let back_off = |id| match BAD_QUERY_RATE_LIMITER.write().unwrap().entry(id) {
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
                    )? {
                        master_key.signatures.extend(our_master_key.signatures);
                    }
                    let json = serde_json::to_value(master_key).expect("to_value always works");
                    let raw =
                        serde_json::from_value(json).expect("RawJson::from_value always works");
                    crate::user::add_cross_signing_keys(
                        &user_id, &raw, &None, &None,
                        false, /* Dont notify. A notification would trigger another key request
                               * resulting in an endless loop */
                    )?;
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
                crate::user::claim_one_time_key(user_id, device_id, key_algorithm)?
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

pub fn get_master_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("master"))
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;
    if let Some(key_data) = key_data {
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub fn get_allowed_master_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &dyn Fn(&UserId) -> bool,
) -> AppResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("master"))
        .order_by(e2e_cross_signing_keys::id.desc())
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;
    if let Some(mut key_data) = key_data {
        clean_signatures(&mut key_data, sender_id, user_id, allowed_signatures)?;
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub fn get_self_signing_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("self_signing"))
        .order_by(e2e_cross_signing_keys::id.desc())
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;
    if let Some(key_data) = key_data {
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}
pub fn get_allowed_self_signing_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &dyn Fn(&UserId) -> bool,
) -> AppResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("self_signing"))
        .order_by(e2e_cross_signing_keys::id.desc())
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;
    if let Some(mut key_data) = key_data {
        clean_signatures(&mut key_data, sender_id, user_id, allowed_signatures)?;
        Ok(serde_json::from_value(key_data).ok())
    } else {
        Ok(None)
    }
}

pub fn get_user_signing_key(user_id: &UserId) -> AppResult<Option<CrossSigningKey>> {
    e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("user_signing"))
        .order_by(e2e_cross_signing_keys::id.desc())
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .map(|data| serde_json::from_value(data).ok())
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

pub fn add_one_time_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_id: &DeviceKeyId,
    one_time_key: &OneTimeKey,
) -> AppResult<()> {
    diesel::insert_into(e2e_one_time_keys::table)
        .values(&NewDbOneTimeKey {
            user_id: user_id.to_owned(),
            device_id: device_id.to_owned(),
            algorithm: key_id.algorithm().to_string(),
            key_id: key_id.to_owned(),
            key_data: serde_json::to_value(one_time_key).unwrap(),
            created_at: UnixMillis::now(),
        })
        .on_conflict((
            e2e_one_time_keys::user_id,
            e2e_one_time_keys::device_id,
            e2e_one_time_keys::algorithm,
            e2e_one_time_keys::key_id,
        ))
        .do_update()
        .set(e2e_one_time_keys::key_data.eq(serde_json::to_value(one_time_key).unwrap()))
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn claim_one_time_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_algorithm: &DeviceKeyAlgorithm,
) -> AppResult<Option<(OwnedDeviceKeyId, OneTimeKey)>> {
    let one_time_key = e2e_one_time_keys::table
        .filter(e2e_one_time_keys::user_id.eq(user_id))
        .filter(e2e_one_time_keys::device_id.eq(device_id))
        .filter(e2e_one_time_keys::algorithm.eq(key_algorithm.as_ref()))
        .order(e2e_one_time_keys::id.asc())
        .first::<DbOneTimeKey>(&mut connect()?)
        .optional()?;
    if let Some(DbOneTimeKey {
        id,
        key_id,
        key_data,
        ..
    }) = one_time_key
    {
        diesel::delete(e2e_one_time_keys::table.find(id)).execute(&mut connect()?)?;
        Ok(Some((
            key_id,
            serde_json::from_value::<OneTimeKey>(key_data)?,
        )))
    } else {
        Ok(None)
    }
}

pub fn add_device_keys(
    user_id: &UserId,
    device_id: &DeviceId,
    device_keys: &DeviceKeys,
) -> AppResult<()> {
    data::user::add_device_keys(user_id, device_id, device_keys)?;
    mark_device_key_update(user_id, device_id)?;
    send_device_key_update(user_id, device_id)?;
    Ok(())
}

pub fn add_cross_signing_keys(
    user_id: &UserId,
    master_key: &CrossSigningKey,
    self_signing_key: &Option<CrossSigningKey>,
    user_signing_key: &Option<CrossSigningKey>,
    notify: bool,
) -> AppResult<()> {
    // TODO: Check signatures
    diesel::insert_into(e2e_cross_signing_keys::table)
        .values(NewDbCrossSigningKey {
            user_id: user_id.to_owned(),
            key_type: "master".to_owned(),
            key_data: serde_json::to_value(master_key)?,
        })
        .execute(&mut connect()?)?;

    // Self-signing key
    if let Some(self_signing_key) = self_signing_key {
        let mut self_signing_key_ids = self_signing_key.keys.values();

        let _self_signing_key_id =
            self_signing_key_ids
                .next()
                .ok_or(MatrixError::invalid_param(
                    "Self signing key contained no key.",
                ))?;

        if self_signing_key_ids.next().is_some() {
            return Err(MatrixError::invalid_param(
                "Self signing key contained more than one key.",
            )
            .into());
        }

        diesel::insert_into(e2e_cross_signing_keys::table)
            .values(NewDbCrossSigningKey {
                user_id: user_id.to_owned(),
                key_type: "self_signing".to_owned(),
                key_data: serde_json::to_value(self_signing_key)?,
            })
            .execute(&mut connect()?)?;
    }

    // User-signing key
    if let Some(user_signing_key) = user_signing_key {
        let mut user_signing_key_ids = user_signing_key.keys.values();

        let _user_signing_key_id =
            user_signing_key_ids
                .next()
                .ok_or(MatrixError::invalid_param(
                    "User signing key contained no key.",
                ))?;

        if user_signing_key_ids.next().is_some() {
            return Err(MatrixError::invalid_param(
                "User signing key contained more than one key.",
            )
            .into());
        }

        diesel::insert_into(e2e_cross_signing_keys::table)
            .values(NewDbCrossSigningKey {
                user_id: user_id.to_owned(),
                key_type: "user_signing".to_owned(),
                key_data: serde_json::to_value(user_signing_key)?,
            })
            .execute(&mut connect()?)?;
    }

    if notify {
        mark_signing_key_update(user_id)?;
    }

    Ok(())
}

pub fn sign_key(
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

    diesel::insert_into(e2e_cross_signing_sigs::table)
        .values(NewDbCrossSignature {
            origin_user_id: sender_id.to_owned(),
            origin_key_id,
            target_user_id: target_user_id.to_owned(),
            target_device_id: OwnedDeviceId::from(target_device_id),
            signature: signature.1,
        })
        .execute(&mut connect()?)?;
    mark_signing_key_update(target_user_id)
}

pub fn mark_signing_key_update(user_id: &UserId) -> AppResult<()> {
    let changed_at = UnixMillis::now();

    let joined_rooms = data::user::joined_rooms(user_id)?;
    for room_id in &joined_rooms {
        // // Don't send key updates to unencrypted rooms
        // if state::get_state(&room_id, &StateEventType::RoomEncryption, "")?.is_none() {
        //     continue;
        // }

        let change = NewDbKeyChange {
            user_id: user_id.to_owned(),
            room_id: Some(room_id.to_owned()),
            changed_at,
            occur_sn: data::next_sn()?,
        };

        diesel::delete(
            e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(user_id))
                .filter(e2e_key_changes::room_id.eq(room_id)),
        )
        .execute(&mut connect()?)?;
        diesel::insert_into(e2e_key_changes::table)
            .values(&change)
            .execute(&mut connect()?)?;
    }

    let change = NewDbKeyChange {
        user_id: user_id.to_owned(),
        room_id: None,
        changed_at,
        occur_sn: data::next_sn()?,
    };

    diesel::delete(
        e2e_key_changes::table
            .filter(e2e_key_changes::user_id.eq(user_id))
            .filter(e2e_key_changes::room_id.is_null()),
    )
    .execute(&mut connect()?)?;
    diesel::insert_into(e2e_key_changes::table)
        .values(&change)
        .execute(&mut connect()?)?;

    if user_id.is_local() {
        let remote_servers = room_joined_servers::table
            .filter(room_joined_servers::room_id.eq_any(joined_rooms))
            .select(room_joined_servers::server_id)
            .distinct()
            .load::<OwnedServerName>(&mut connect()?)?;

        let content = SigningKeyUpdateContent::new(user_id.to_owned());
        let edu = Edu::SigningKeyUpdate(content);

        let _ = sending::send_edu_servers(remote_servers.into_iter(), &edu);
    }

    Ok(())
}

pub fn mark_device_key_update(user_id: &UserId, _device_id: &DeviceId) -> AppResult<()> {
    let changed_at = UnixMillis::now();
    let joined_rooms = data::user::joined_rooms(user_id)?;
    let occur_sn = data::next_sn()?;
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

        diesel::delete(
            e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(user_id))
                .filter(e2e_key_changes::room_id.eq(room_id)),
        )
        .execute(&mut connect()?)?;
        diesel::insert_into(e2e_key_changes::table)
            .values(&change)
            .execute(&mut connect()?)?;
    }

    let change = NewDbKeyChange {
        user_id: user_id.to_owned(),
        room_id: None,
        changed_at,
        occur_sn,
    };

    diesel::delete(
        e2e_key_changes::table
            .filter(e2e_key_changes::user_id.eq(user_id))
            .filter(e2e_key_changes::room_id.is_null()),
    )
    .execute(&mut connect()?)?;
    diesel::insert_into(e2e_key_changes::table)
        .values(&change)
        .execute(&mut connect()?)?;

    Ok(())
}

pub fn mark_device_key_update_with_joined_rooms(
    user_id: &UserId,
    _device_id: &DeviceId,
    joined_rooms: &[OwnedRoomId],
) -> AppResult<()> {
    let changed_at = UnixMillis::now();
    let occur_sn = data::next_sn()?;
    for room_id in joined_rooms {
        let change = NewDbKeyChange {
            user_id: user_id.to_owned(),
            room_id: Some(room_id.to_owned()),
            changed_at,
            occur_sn,
        };

        diesel::delete(
            e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(user_id))
                .filter(e2e_key_changes::room_id.eq(room_id)),
        )
        .execute(&mut connect()?)?;
        diesel::insert_into(e2e_key_changes::table)
            .values(&change)
            .execute(&mut connect()?)?;
    }
    Ok(())
}

pub fn send_device_key_update(user_id: &UserId, device_id: &DeviceId) -> AppResult<()> {
    let joined_rooms = data::user::joined_rooms(user_id)?;
    send_device_key_update_with_joined_rooms(user_id, device_id, &joined_rooms)
}

fn send_device_key_update_with_joined_rooms(
    user_id: &UserId,
    device_id: &DeviceId,
    joined_rooms: &[OwnedRoomId],
) -> AppResult<()> {
    if user_id.is_remote() {
        return Ok(());
    }
    let remote_servers = room_joined_servers::table
        .filter(room_joined_servers::room_id.eq_any(joined_rooms))
        .select(room_joined_servers::server_id)
        .distinct()
        .load::<OwnedServerName>(&mut connect()?)?;

    let content = DeviceListUpdateContent::new(
        user_id.to_owned(),
        device_id.to_owned(),
        data::next_sn()? as u64,
    );
    let edu = Edu::DeviceListUpdate(content);

    sending::send_edu_servers(remote_servers.into_iter(), &edu)
}
