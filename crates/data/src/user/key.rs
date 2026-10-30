use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::encryption::{CrossSigningKey, DeviceKeys, OneTimeKey};
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::{DeviceKeyAlgorithm, Seqnum, UnixMillis};
use crate::schema::*;
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
    pub id: i64,

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

pub async fn count_one_time_keys(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<BTreeMap<DeviceKeyAlgorithm, u64>> {
    let list = e2e_one_time_keys::table
        .filter(e2e_one_time_keys::user_id.eq(user_id))
        .filter(e2e_one_time_keys::device_id.eq(device_id))
        .group_by(e2e_one_time_keys::algorithm)
        .select((e2e_one_time_keys::algorithm, diesel::dsl::count_star()))
        .load::<(String, i64)>(&mut connect().await?)
        .await?;
    Ok(BTreeMap::from_iter(
        list.into_iter()
            .map(|(k, v)| (DeviceKeyAlgorithm::from(k), v as u64)),
    ))
}

pub async fn add_device_keys(
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
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn get_device_keys(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<Option<DeviceKeys>> {
    e2e_device_keys::table
        .filter(e2e_device_keys::user_id.eq(user_id))
        .filter(e2e_device_keys::device_id.eq(device_id))
        .select(e2e_device_keys::key_data)
        .first::<JsonValue>(&mut connect().await?)
        .await
        .optional()?
        .map(|v| serde_json::from_value(v).map_err(Into::into))
        .transpose()
}

pub async fn all_device_key_ids(user_id: &UserId) -> DataResult<Vec<OwnedDeviceId>> {
    e2e_device_keys::table
        .filter(e2e_device_keys::user_id.eq(user_id))
        .select(e2e_device_keys::device_id)
        .load::<OwnedDeviceId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn get_device_keys_and_sigs(
    user_id: &UserId,
    device_id: &DeviceId,
) -> DataResult<Option<DeviceKeys>> {
    let Some(mut device_keys) = get_device_keys(user_id, device_id).await? else {
        return Ok(None);
    };
    let signatures = e2e_cross_signing_sigs::table
        .filter(e2e_cross_signing_sigs::origin_user_id.eq(user_id))
        .filter(e2e_cross_signing_sigs::target_user_id.eq(user_id))
        .filter(e2e_cross_signing_sigs::target_device_id.eq(device_id))
        .load::<DbCrossSignature>(&mut connect().await?)
        .await?;
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

pub async fn delete_device_key_material(user_id: &UserId, device_id: &DeviceId) -> DataResult<()> {
    let mut conn = connect().await?;

    diesel::delete(
        e2e_device_keys::table
            .filter(e2e_device_keys::user_id.eq(user_id))
            .filter(e2e_device_keys::device_id.eq(device_id)),
    )
    .execute(&mut conn)
    .await?;

    diesel::delete(
        e2e_one_time_keys::table
            .filter(e2e_one_time_keys::user_id.eq(user_id))
            .filter(e2e_one_time_keys::device_id.eq(device_id)),
    )
    .execute(&mut conn)
    .await?;

    diesel::delete(
        e2e_fallback_keys::table
            .filter(e2e_fallback_keys::user_id.eq(user_id))
            .filter(e2e_fallback_keys::device_id.eq(device_id)),
    )
    .execute(&mut conn)
    .await?;

    // Drop cross-signing signatures targeting this device so a later reuse of the
    // same device ID cannot resurrect stale signatures over fresh device keys.
    diesel::delete(
        e2e_cross_signing_sigs::table
            .filter(e2e_cross_signing_sigs::target_user_id.eq(user_id))
            .filter(e2e_cross_signing_sigs::target_device_id.eq(device_id)),
    )
    .execute(&mut conn)
    .await?;

    Ok(())
}

pub async fn delete_all_device_keys(user_id: &UserId) -> DataResult<()> {
    let mut conn = connect().await?;
    let device_ids = e2e_device_keys::table
        .filter(e2e_device_keys::user_id.eq(user_id))
        .select(e2e_device_keys::device_id)
        .load::<OwnedDeviceId>(&mut conn)
        .await?;

    if !device_ids.is_empty() {
        diesel::delete(
            e2e_cross_signing_sigs::table
                .filter(e2e_cross_signing_sigs::target_user_id.eq(user_id))
                .filter(e2e_cross_signing_sigs::target_device_id.eq_any(&device_ids)),
        )
        .execute(&mut conn)
        .await?;
    }
    diesel::delete(e2e_device_keys::table.filter(e2e_device_keys::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;
    diesel::delete(e2e_one_time_keys::table.filter(e2e_one_time_keys::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;
    diesel::delete(e2e_fallback_keys::table.filter(e2e_fallback_keys::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;
    Ok(())
}

pub async fn keys_changed_users(
    user_id: &UserId,
    since_sn: Seqnum,
    until_sn: Option<Seqnum>,
) -> DataResult<Vec<OwnedUserId>> {
    let room_ids = crate::user::joined_rooms(user_id).await?;
    if let Some(until_sn) = until_sn {
        e2e_key_changes::table
            .filter(
                e2e_key_changes::room_id
                    .eq_any(&room_ids)
                    .or(e2e_key_changes::user_id.eq(user_id)),
            )
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .filter(e2e_key_changes::occur_sn.le(until_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect().await?)
            .await
            .map_err(Into::into)
    } else {
        e2e_key_changes::table
            .filter(
                e2e_key_changes::room_id
                    .eq_any(&room_ids)
                    .or(e2e_key_changes::user_id.eq(user_id)),
            )
            .filter(e2e_key_changes::occur_sn.ge(since_sn))
            .select(e2e_key_changes::user_id)
            .load::<OwnedUserId>(&mut connect().await?)
            .await
            .map_err(Into::into)
    }
}

/// Check if user has a master cross-signing key
pub async fn has_master_cross_signing_key(user_id: &UserId) -> DataResult<bool> {
    let count = e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq("master"))
        .count()
        .get_result::<i64>(&mut connect().await?)
        .await?;
    Ok(count > 0)
}

/// Set the timestamp until which cross-signing key replacement is allowed without UIA
pub async fn set_cross_signing_replacement_allowed(
    user_id: &UserId,
    expires_ts: i64,
) -> DataResult<()> {
    diesel::insert_into(e2e_cross_signing_uia_bypass::table)
        .values((
            e2e_cross_signing_uia_bypass::user_id.eq(user_id),
            e2e_cross_signing_uia_bypass::updatable_before_ts.eq(expires_ts),
        ))
        .on_conflict(e2e_cross_signing_uia_bypass::user_id)
        .do_update()
        .set(e2e_cross_signing_uia_bypass::updatable_before_ts.eq(expires_ts))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Get the timestamp until which cross-signing key replacement is allowed without UIA
pub async fn get_cross_signing_replacement_allowed(user_id: &UserId) -> DataResult<Option<i64>> {
    e2e_cross_signing_uia_bypass::table
        .filter(e2e_cross_signing_uia_bypass::user_id.eq(user_id))
        .select(e2e_cross_signing_uia_bypass::updatable_before_ts)
        .first::<i64>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Return the raw `key_data` JSON of the latest cross-signing key of a kind.
pub async fn get_cross_signing_key(
    user_id: &UserId,
    key_type: &str,
) -> DataResult<Option<JsonValue>> {
    e2e_cross_signing_keys::table
        .filter(e2e_cross_signing_keys::user_id.eq(user_id))
        .filter(e2e_cross_signing_keys::key_type.eq(key_type))
        .order_by(e2e_cross_signing_keys::id.desc())
        .select(e2e_cross_signing_keys::key_data)
        .first::<JsonValue>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Return the latest cross-signing key of a kind with uploaded signatures attached.
pub async fn get_cross_signing_key_and_sigs<F>(
    user_id: &UserId,
    key_type: &str,
    allowed_signature_origin: F,
) -> DataResult<Option<JsonValue>>
where
    F: Fn(&UserId) -> bool,
{
    let Some(key_data) = get_cross_signing_key(user_id, key_type).await? else {
        return Ok(None);
    };

    let mut key = serde_json::from_value::<CrossSigningKey>(key_data)?;
    for target_key_id in key.keys.keys() {
        let signatures = e2e_cross_signing_sigs::table
            .filter(e2e_cross_signing_sigs::target_user_id.eq(user_id))
            .filter(
                e2e_cross_signing_sigs::target_device_id
                    .eq(OwnedDeviceId::from(target_key_id.key_name().as_str())),
            )
            .load::<DbCrossSignature>(&mut connect().await?)
            .await?;
        for DbCrossSignature {
            origin_user_id,
            origin_key_id,
            signature,
            ..
        } in signatures
        {
            if !allowed_signature_origin(&origin_user_id) {
                continue;
            }
            key.signatures
                .entry(origin_user_id)
                .or_default()
                .insert(origin_key_id, signature);
        }
    }

    serde_json::to_value(key).map(Some).map_err(Into::into)
}

/// Insert a cross-signing key of the given kind.
pub async fn add_cross_signing_key(
    user_id: &UserId,
    key_type: &str,
    key: &CrossSigningKey,
) -> DataResult<()> {
    diesel::insert_into(e2e_cross_signing_keys::table)
        .values(NewDbCrossSigningKey {
            user_id: user_id.to_owned(),
            key_type: key_type.to_owned(),
            key_data: serde_json::to_value(key)?,
        })
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Store a validated signature idempotently, including a retry after recovery.
pub async fn add_cross_signing_sig(signature: NewDbCrossSignature) -> DataResult<()> {
    diesel::insert_into(e2e_cross_signing_sigs::table)
        .values(signature)
        .on_conflict((
            e2e_cross_signing_sigs::origin_user_id,
            e2e_cross_signing_sigs::origin_key_id,
            e2e_cross_signing_sigs::target_user_id,
            e2e_cross_signing_sigs::target_device_id,
        ))
        .do_update()
        .set(
            e2e_cross_signing_sigs::signature
                .eq(diesel::upsert::excluded(e2e_cross_signing_sigs::signature)),
        )
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Insert or replace a one-time key for a device.
pub async fn add_one_time_key(
    user_id: &UserId,
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
            key_data: serde_json::to_value(one_time_key)?,
            created_at: UnixMillis::now(),
        })
        .on_conflict((
            e2e_one_time_keys::user_id,
            e2e_one_time_keys::device_id,
            e2e_one_time_keys::algorithm,
            e2e_one_time_keys::key_id,
        ))
        .do_update()
        .set(e2e_one_time_keys::key_data.eq(serde_json::to_value(one_time_key)?))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Insert or replace a fallback key for a device and algorithm.
pub async fn add_fallback_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_id: &DeviceKeyId,
    fallback_key: &OneTimeKey,
) -> DataResult<()> {
    diesel::delete(
        e2e_fallback_keys::table
            .filter(e2e_fallback_keys::user_id.eq(user_id))
            .filter(e2e_fallback_keys::device_id.eq(device_id))
            .filter(e2e_fallback_keys::algorithm.eq(key_id.algorithm().to_string())),
    )
    .execute(&mut connect().await?)
    .await?;

    diesel::insert_into(e2e_fallback_keys::table)
        .values(&NewDbFallbackKey {
            user_id: user_id.to_owned(),
            device_id: device_id.to_owned(),
            algorithm: key_id.algorithm().to_string(),
            key_id: key_id.to_owned(),
            key_data: serde_json::to_value(fallback_key)?,
            used_at: None,
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Claim (read and remove) the oldest one-time key for a device and algorithm.
pub async fn claim_one_time_key(
    user_id: &UserId,
    device_id: &DeviceId,
    key_algorithm: &DeviceKeyAlgorithm,
) -> DataResult<Option<(OwnedDeviceKeyId, OneTimeKey)>> {
    let one_time_key = e2e_one_time_keys::table
        .filter(e2e_one_time_keys::user_id.eq(user_id))
        .filter(e2e_one_time_keys::device_id.eq(device_id))
        .filter(e2e_one_time_keys::algorithm.eq(key_algorithm.as_ref()))
        .order(e2e_one_time_keys::id.asc())
        .first::<DbOneTimeKey>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(DbOneTimeKey {
        id,
        key_id,
        key_data,
        ..
    }) = one_time_key
    {
        diesel::delete(e2e_one_time_keys::table.find(id))
            .execute(&mut connect().await?)
            .await?;
        Ok(Some((
            key_id,
            serde_json::from_value::<OneTimeKey>(key_data)?,
        )))
    } else if let Some(DbFallbackKey {
        id,
        key_id,
        key_data,
        used_at,
        ..
    }) = e2e_fallback_keys::table
        .filter(e2e_fallback_keys::user_id.eq(user_id))
        .filter(e2e_fallback_keys::device_id.eq(device_id))
        .filter(e2e_fallback_keys::algorithm.eq(key_algorithm.as_ref()))
        .order(e2e_fallback_keys::id.desc())
        .first::<DbFallbackKey>(&mut connect().await?)
        .await
        .optional()?
    {
        if used_at.is_none() {
            diesel::update(e2e_fallback_keys::table.find(id))
                .set(e2e_fallback_keys::used_at.eq(UnixMillis::now().get() as i64))
                .execute(&mut connect().await?)
                .await?;
        }
        Ok(Some((
            key_id,
            serde_json::from_value::<OneTimeKey>(key_data)?,
        )))
    } else {
        Ok(None)
    }
}

/// The persisted revision used by device-list EDUs and federation resyncs.
pub async fn device_list_stream_id(user_id: &UserId) -> DataResult<Seqnum> {
    Ok(device_streams::table
        .filter(device_streams::user_id.eq(user_id))
        .select(diesel::dsl::max(device_streams::id))
        .first::<Option<i64>>(&mut connect().await?)
        .await?
        .unwrap_or_default())
}

/// Atomically retain the newest marker for a `(user, room)` (or global when
/// `change.room_id` is `None`), including out-of-order concurrent notifications.
pub async fn replace_key_change(change: &NewDbKeyChange) -> DataResult<()> {
    use diesel::query_dsl::methods::FilterDsl;
    diesel::insert_into(e2e_key_changes::table)
        .values(change)
        .on_conflict((e2e_key_changes::user_id, e2e_key_changes::room_id))
        .do_update()
        .set((
            e2e_key_changes::occur_sn.eq(diesel::upsert::excluded(e2e_key_changes::occur_sn)),
            e2e_key_changes::changed_at.eq(diesel::upsert::excluded(e2e_key_changes::changed_at)),
        ))
        // A slower, older notification cannot move the stream cursor backwards.
        .filter(e2e_key_changes::occur_sn.lt(diesel::upsert::excluded(e2e_key_changes::occur_sn)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires a disposable PostgreSQL database; run explicitly in CI"]
    async fn cross_signing_retries_and_key_change_markers_are_atomic() {
        let url = std::env::var("PALPO_TEST_DATABASE_URL").expect("set PALPO_TEST_DATABASE_URL");
        assert!(
            url::Url::parse(&url)
                .unwrap()
                .path()
                .starts_with("/palpo_crypto_test_"),
            "use a disposable crypto regression database"
        );
        let config: crate::DbConfig =
            serde_json::from_value(serde_json::json!({"url":url,"pool_size":10})).unwrap();
        tokio::task::spawn_blocking(move || crate::init(&config))
            .await
            .unwrap()
            .unwrap();
        let user =
            UserId::parse(format!("@crypto_{}:example.test", rand::random::<u64>())).unwrap();
        let mut signature = NewDbCrossSignature {
            origin_user_id: user.clone(),
            origin_key_id: DeviceKeyId::parse("ed25519:SIGNER").unwrap(),
            target_user_id: user.clone(),
            target_device_id: "DEVICE".into(),
            signature: "verified-one".into(),
        };
        add_cross_signing_sig(signature.clone()).await.unwrap();
        add_cross_signing_sig(signature.clone())
            .await
            .expect("a repeated signature is a successful retry");
        signature.signature = "verified-two".into();
        add_cross_signing_sig(signature.clone()).await.unwrap();
        let mut jobs = Vec::new();
        for _ in 0..32 {
            let signature = signature.clone();
            jobs.push(tokio::spawn(async move {
                add_cross_signing_sig(signature).await.unwrap();
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        let signatures = e2e_cross_signing_sigs::table
            .filter(e2e_cross_signing_sigs::origin_user_id.eq(&user))
            .select(e2e_cross_signing_sigs::signature)
            .load::<String>(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(signatures, vec!["verified-two"]);

        for room in [None, Some(RoomId::parse("!crypto:example.test").unwrap())] {
            let mut jobs = Vec::new();
            for sequence in 100..132 {
                let change = NewDbKeyChange {
                    user_id: user.clone(),
                    room_id: room.clone(),
                    occur_sn: sequence,
                    changed_at: UnixMillis(sequence as u64),
                };
                jobs.push(tokio::spawn(async move {
                    replace_key_change(&change).await.unwrap();
                }));
            }
            for job in jobs {
                job.await.unwrap();
            }
            // An old request completing last must not hide a newer change from
            // a client's since-stream query.
            replace_key_change(&NewDbKeyChange {
                user_id: user.clone(),
                room_id: room.clone(),
                occur_sn: 99,
                changed_at: UnixMillis(99),
            })
            .await
            .unwrap();
            replace_key_change(&NewDbKeyChange {
                user_id: user.clone(),
                room_id: room.clone(),
                occur_sn: 131,
                changed_at: UnixMillis(131),
            })
            .await
            .unwrap();
            let rows = e2e_key_changes::table
                .filter(e2e_key_changes::user_id.eq(&user))
                .select((
                    e2e_key_changes::room_id,
                    e2e_key_changes::occur_sn,
                    e2e_key_changes::changed_at,
                ))
                .load::<(Option<OwnedRoomId>, i64, UnixMillis)>(&mut connect().await.unwrap())
                .await
                .unwrap();
            let matches = rows
                .iter()
                .filter(|(candidate, ..)| candidate == &room)
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].1, 131);
            assert_eq!(matches[0].2, UnixMillis(131));
        }
        diesel::delete(e2e_key_changes::table.filter(e2e_key_changes::user_id.eq(&user)))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        diesel::delete(
            e2e_cross_signing_sigs::table.filter(e2e_cross_signing_sigs::origin_user_id.eq(&user)),
        )
        .execute(&mut connect().await.unwrap())
        .await
        .unwrap();
    }
}
