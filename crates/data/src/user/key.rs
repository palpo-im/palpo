use std::collections::BTreeMap;

use diesel::prelude::*;

use crate::core::encryption::{CrossSigningKey, DeviceKeys, OneTimeKey};
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::{DeviceKeyAlgorithm, Seqnum, UnixMillis};
use crate::schema::*;
use crate::user::clean_signatures;
use crate::{DataResult, connect};

#[derive(Identifiable, Insertable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_cross_signing_keys)]
pub struct DbCrossSigningKey {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub key_type: String,
    pub key_data: JsonValue,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = e2e_cross_signing_keys)]
pub struct NewDbCrossSigningKey {
    pub user_id: OwnedUserId,
    pub key_type: String,
    pub key_data: JsonValue,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_cross_signing_sigs)]
pub struct DbCrossSignature {
    pub id: i64,

    pub origin_user_id: OwnedUserId,
    pub origin_key_id: OwnedDeviceKeyId,
    pub target_user_id: OwnedUserId,
    pub target_device_id: OwnedDeviceId,
    pub signature: String,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = e2e_cross_signing_sigs)]
pub struct NewDbCrossSignature {
    pub origin_user_id: OwnedUserId,
    pub origin_key_id: OwnedDeviceKeyId,
    pub target_user_id: OwnedUserId,
    pub target_device_id: OwnedDeviceId,
    pub signature: String,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_fallback_keys)]
pub struct DbFallbackKey {
    pub id: String,

    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub algorithm: String,
    pub key_id: OwnedDeviceKeyId,
    pub key_data: JsonValue,
    pub used_at: Option<i64>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = e2e_fallback_keys)]
pub struct NewDbFallbackKey {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub algorithm: String,
    pub key_id: OwnedDeviceKeyId,
    pub key_data: JsonValue,
    pub used_at: Option<i64>,
    pub created_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_one_time_keys)]
pub struct DbOneTimeKey {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub algorithm: String,
    pub key_id: OwnedDeviceKeyId,
    pub key_data: JsonValue,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = e2e_one_time_keys)]
pub struct NewDbOneTimeKey {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub algorithm: String,
    pub key_id: OwnedDeviceKeyId,
    pub key_data: JsonValue,
    pub created_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_device_keys)]
pub struct DbDeviceKey {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub algorithm: String,
    pub stream_id: i64,
    pub display_name: Option<String>,
    pub key_data: JsonValue,
    pub created_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = e2e_device_keys)]
pub struct NewDbDeviceKey {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub stream_id: i64,
    pub display_name: Option<String>,
    pub key_data: JsonValue,
    pub created_at: UnixMillis,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = e2e_key_changes)]
pub struct DbKeyChange {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub room_id: Option<OwnedRoomId>,
    pub occur_sn: i64,
    pub changed_at: UnixMillis,
}
#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = e2e_key_changes)]
pub struct NewDbKeyChange {
    pub user_id: OwnedUserId,
    pub room_id: Option<OwnedRoomId>,
    pub occur_sn: i64,
    pub changed_at: UnixMillis,
}

pub fn get_master_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &dyn Fn(&UserId) -> bool,
) -> DataResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("master"))
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

pub fn get_self_signing_key(
    sender_id: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &dyn Fn(&UserId) -> bool,
) -> DataResult<Option<CrossSigningKey>> {
    let key_data = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("self_signing"))
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
pub fn get_user_signing_key(user_id: &OwnedUserId) -> DataResult<Option<CrossSigningKey>> {
    e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("user_signing"))
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .map(|data| serde_json::from_value(data).ok())
        .optional()
        .map(|v| v.flatten())
        .map_err(Into::into)
}

pub fn add_one_time_key(
    user_id: &OwnedUserId,
    device_id: &DeviceId,
    key_id: &DeviceKeyId,
    one_time_key: &OneTimeKey,
) -> DataResult<()> {
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
    user_id: &OwnedUserId,
    device_id: &DeviceId,
    key_algorithm: &DeviceKeyAlgorithm,
) -> DataResult<Option<(OwnedDeviceKeyId, OneTimeKey)>> {
    let one_time_key = e2e_one_time_keys::table
        .filter(e2e_one_time_keys::user_id.eq(user_id))
        .filter(e2e_one_time_keys::device_id.eq(device_id))
        .filter(e2e_one_time_keys::algorithm.eq(key_algorithm.as_ref()))
        .order(e2e_one_time_keys::id.desc())
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

pub fn count_one_time_keys(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<BTreeMap<DeviceKeyAlgorithm, u64>> {
    let list = e2e_one_time_keys::table
        .filter(e2e_one_time_keys::user_id.eq(user_id))
        .filter(e2e_one_time_keys::device_id.eq(device_id))
        .group_by(e2e_one_time_keys::algorithm)
        .select((e2e_one_time_keys::algorithm, diesel::dsl::count_star()))
        .load::<(String, i64)>(&mut connect()?)?;
    Ok(BTreeMap::from_iter(
        list.into_iter()
            .map(|(k, v)| (DeviceKeyAlgorithm::from(k), v as u64)),
    ))
}

pub fn add_device_keys(
    user_id: &UserId,
    device_id: &DeviceId,
    device_keys: &DeviceKeys,
) -> DataResult<()> {
    let new_device_key = NewDbDeviceKey {
        user_id: user_id.to_owned(),
        device_id: device_id.to_owned(),
        stream_id: 0,
        display_name: device_keys.unsigned.device_display_name.clone(),
        key_data: serde_json::to_value(device_keys).unwrap(),
        created_at: UnixMillis::now(),
    };
    diesel::insert_into(e2e_device_keys::table)
        .values(&new_device_key)
        .on_conflict((e2e_device_keys::user_id, e2e_device_keys::device_id))
        .do_update()
        .set(&new_device_key)
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn get_device_keys(user_id: &UserId, device_id: &DeviceId) -> DataResult<Option<DeviceKeys>> {
    e2e_device_keys::table
        .filter(e2e_device_keys::user_id.eq(user_id))
        .filter(e2e_device_keys::device_id.eq(device_id))
        .select(e2e_device_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?
        .map(|v| serde_json::from_value(v).map_err(Into::into))
        .transpose()
}

pub fn get_device_keys_and_sigs(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<Option<DeviceKeys>> {
    let Some(mut device_keys) = get_device_keys(user_id, device_id)? else {
        return Ok(None);
    };
    let signatures = e2e_cross_signing_sigs::table
        .filter(e2e_cross_signing_sigs::origin_user_id.eq(user_id))
        .filter(e2e_cross_signing_sigs::target_user_id.eq(user_id))
        .filter(e2e_cross_signing_sigs::target_device_id.eq(device_id))
        .load::<DbCrossSignature>(&mut connect()?)?;
    for DbCrossSignature {
        origin_key_id,
        signature,
        ..
    } in signatures
    {
        device_keys
            .signatures
            .entry(user_id.to_owned())
            .or_default()
            .insert(origin_key_id, signature);
    }
    Ok(Some(device_keys))
}

pub fn keys_changed_users(
    user_id: &UserId,
    since_sn: Seqnum,
    until_sn: Option<Seqnum>,
) -> DataResult<Vec<OwnedUserId>> {
    let room_ids = crate::user::joined_rooms(user_id)?;
    if let Some(until_sn) = until_sn {
        e2e_key_changes::table
            .filter(e2e_key_changes::room_id.eq_any(&room_ids))
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .filter(e2e_key_changes::occur_sn.le(until_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect()?)
            .map_err(Into::into)
    } else {
        e2e_key_changes::table
            .filter(e2e_key_changes::room_id.eq_any(&room_ids))
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect()?)
            .map_err(Into::into)
    }
}
