use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::client::device::Device;
use crate::core::events::AnyToDeviceEvent;
use crate::core::identifiers::*;
use crate::core::serde::{JsonValue, RawJson};
use crate::core::{MatrixError, Seqnum, UnixMillis};
use crate::schema::*;
use crate::user::{NewDbAccessToken, NewDbRefreshToken};
use crate::{DataError, DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_devices)]
pub struct DbUserDevice {
    pub id: i64,

    pub user_id: OwnedUserId,

    pub device_id: OwnedDeviceId,

    /// Public display name of the device.
    pub display_name: Option<String>,

    pub user_agent: Option<String>,

    pub is_hidden: bool,
    /// Most recently seen IP address of the session.
    pub last_seen_ip: Option<String>,

    /// Unix timestamp that the session was last active.
    pub last_seen_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_devices)]
pub struct NewDbUserDevice {
    pub user_id: OwnedUserId,

    pub device_id: OwnedDeviceId,

    /// Public display name of the device.
    pub display_name: Option<String>,

    pub user_agent: Option<String>,

    pub is_hidden: bool,
    /// Most recently seen IP address of the session.
    pub last_seen_ip: Option<String>,

    /// Unix timestamp that the session was last active.
    pub last_seen_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
}

impl DbUserDevice {
    pub fn into_matrix_device(self) -> Device {
        let Self {
            device_id,
            display_name,
            last_seen_at,
            last_seen_ip,
            ..
        } = self;
        Device {
            device_id,
            display_name,
            last_seen_ip,
            last_seen_ts: last_seen_at,
        }
    }
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = device_inboxes)]
pub struct DbDeviceInbox {
    pub id: i64,

    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub json_data: JsonValue,
    pub occur_sn: i64,
    pub created_at: i64,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = device_inboxes)]
pub struct NewDbDeviceInbox {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub json_data: JsonValue,
    pub created_at: i64,
}

pub async fn create_device(
    user_id: &UserId,
    device_id: &DeviceId,
    token: &str,
    initial_device_display_name: Option<String>,
    last_seen_ip: Option<String>,
) -> DataResult<DbUserDevice> {
    let device = diesel::insert_into(user_devices::table)
        .values(NewDbUserDevice {
            user_id: user_id.to_owned(),
            device_id: device_id.to_owned(),
            display_name: initial_device_display_name,
            user_agent: None,
            is_hidden: false,
            last_seen_ip,
            last_seen_at: Some(UnixMillis::now()),
            created_at: UnixMillis::now(),
        })
        .get_result(&mut connect().await?)
        .await?;

    diesel::insert_into(user_access_tokens::table)
        .values(NewDbAccessToken::new(
            user_id.to_owned(),
            device_id.to_owned(),
            token.to_owned(),
            None,
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(device)
}

pub async fn get_device(user_id: &UserId, device_id: &DeviceId) -> DataResult<DbUserDevice> {
    user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .filter(user_devices::device_id.eq(device_id))
        .first::<DbUserDevice>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

#[derive(AsChangeset, Default, Debug)]
#[diesel(table_name = user_devices)]
struct DbUserDeviceChanges {
    display_name: Option<Option<String>>,
    user_agent: Option<Option<String>>,
    last_seen_ip: Option<Option<String>>,
    last_seen_at: Option<Option<UnixMillis>>,
}

pub struct DeviceUpdate {
    pub display_name: Option<Option<String>>,
    pub user_agent: Option<Option<String>>,
    pub last_seen_ip: Option<Option<String>>,
    pub last_seen_at: Option<Option<UnixMillis>>,
}

impl From<DeviceUpdate> for DbUserDeviceChanges {
    fn from(value: DeviceUpdate) -> Self {
        Self {
            display_name: value.display_name,
            user_agent: value.user_agent,
            last_seen_ip: value.last_seen_ip,
            last_seen_at: value.last_seen_at,
        }
    }
}

pub async fn update_device(
    user_id: &UserId,
    device_id: &DeviceId,
    update: DeviceUpdate,
) -> DataResult<DbUserDevice> {
    let changes: DbUserDeviceChanges = update.into();
    diesel::update(
        user_devices::table
            .filter(user_devices::user_id.eq(user_id))
            .filter(user_devices::device_id.eq(device_id)),
    )
    .set(changes)
    .get_result::<DbUserDevice>(&mut connect().await?)
    .await
    .map_err(Into::into)
}

pub async fn get_devices(user_id: &UserId) -> DataResult<Vec<DbUserDevice>> {
    user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .load::<DbUserDevice>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn is_device_exists(user_id: &UserId, device_id: &DeviceId) -> DataResult<bool> {
    let query = user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .filter(user_devices::device_id.eq(device_id));
    diesel_exists!(query, &mut connect().await?).map_err(Into::into)
}

pub async fn remove_device(user_id: &UserId, device_id: &DeviceId) -> DataResult<()> {
    let count = diesel::delete(
        user_devices::table
            .filter(user_devices::user_id.eq(user_id))
            .filter(user_devices::device_id.eq(device_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    if count == 0 {
        if diesel_exists!(
            user_devices::table.filter(user_devices::device_id.eq(device_id)),
            &mut connect().await?
        )? {
            return Err(MatrixError::forbidden("Device not owned by user.", None).into());
        } else {
            return Err(MatrixError::not_found("Device not found.").into());
        }
    }

    delete_access_tokens(user_id, device_id).await?;
    delete_refresh_tokens(user_id, device_id).await?;
    super::pusher::delete_device_pushers(user_id, device_id).await?;
    Ok(())
}

pub async fn set_refresh_token(
    user_id: &UserId,
    device_id: &DeviceId,
    token: &str,
    expires_at: u64,
    ultimate_session_expires_at: u64,
) -> DataResult<i64> {
    let id = connect()
        .await?
        .transaction::<_, DieselError, _>(async |conn| {
            diesel::delete(
                user_refresh_tokens::table
                    .filter(user_refresh_tokens::user_id.eq(user_id))
                    .filter(user_refresh_tokens::device_id.eq(device_id)),
            )
            .execute(conn)
            .await?;
            diesel::insert_into(user_refresh_tokens::table)
                .values(NewDbRefreshToken::new(
                    user_id.to_owned(),
                    device_id.to_owned(),
                    token.to_owned(),
                    expires_at as i64,
                    ultimate_session_expires_at as i64,
                ))
                .returning(user_refresh_tokens::id)
                .get_result::<i64>(conn)
                .await
        })
        .await?;

    Ok(id)
}

pub async fn set_access_token(
    user_id: &UserId,
    device_id: &DeviceId,
    token: &str,
    refresh_token_id: Option<i64>,
) -> DataResult<()> {
    // Capture the token currently bound to this device (if any). The upsert
    // below replaces it, orphaning that string in the DB; its cached
    // authentication must be dropped or it would keep authenticating until the
    // cache TTL (e.g. the pre-rotation token after a refresh).
    let old_token = user_access_tokens::table
        .filter(user_access_tokens::user_id.eq(user_id))
        .filter(user_access_tokens::device_id.eq(device_id))
        .select(user_access_tokens::token)
        .first::<String>(&mut connect().await?)
        .await
        .optional()?;

    diesel::insert_into(user_access_tokens::table)
        .values(NewDbAccessToken::new(
            user_id.to_owned(),
            device_id.to_owned(),
            token.to_owned(),
            refresh_token_id,
        ))
        .on_conflict((user_access_tokens::user_id, user_access_tokens::device_id))
        .do_update()
        .set(user_access_tokens::token.eq(token))
        .execute(&mut connect().await?)
        .await?;

    if let Some(old_token) = old_token
        && old_token != token
    {
        super::access_token::invalidate_token(&old_token);
    }
    Ok(())
}

pub async fn delete_access_tokens(user_id: &UserId, device_id: &DeviceId) -> DataResult<()> {
    diesel::delete(
        user_access_tokens::table
            .filter(user_access_tokens::user_id.eq(user_id))
            .filter(user_access_tokens::device_id.eq(device_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    super::access_token::invalidate_user(user_id);
    Ok(())
}

pub async fn delete_refresh_tokens(user_id: &UserId, device_id: &DeviceId) -> DataResult<()> {
    diesel::delete(
        user_refresh_tokens::table
            .filter(user_refresh_tokens::user_id.eq(user_id))
            .filter(user_refresh_tokens::device_id.eq(device_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

pub async fn get_to_device_events(
    user_id: &UserId,
    device_id: &DeviceId,
    _since_sn: Option<Seqnum>,
    _until_sn: Option<Seqnum>,
) -> DataResult<Vec<RawJson<AnyToDeviceEvent>>> {
    device_inboxes::table
        .filter(device_inboxes::user_id.eq(user_id))
        .filter(device_inboxes::device_id.eq(device_id))
        .load::<DbDeviceInbox>(&mut connect().await?)
        .await?
        .into_iter()
        .map(|event| {
            serde_json::from_value(event.json_data.clone())
                .map_err(|_| DataError::public("Invalid JSON in device inbox"))
        })
        .collect::<DataResult<Vec<_>>>()
}

pub async fn add_to_device_event(
    sender: &UserId,
    target_user_id: &UserId,
    target_device_id: &DeviceId,
    event_type: &str,
    content: serde_json::Value,
) -> DataResult<()> {
    let mut json = serde_json::Map::new();
    json.insert("type".to_owned(), event_type.to_owned().into());
    json.insert("sender".to_owned(), sender.to_string().into());
    json.insert("content".to_owned(), content);

    let json_data = serde_json::to_value(&json)?;

    diesel::insert_into(device_inboxes::table)
        .values(NewDbDeviceInbox {
            user_id: target_user_id.to_owned(),
            device_id: target_device_id.to_owned(),
            json_data,
            created_at: UnixMillis::now().get() as i64,
        })
        .execute(&mut connect().await?)
        .await?;

    Ok(())
}

pub async fn remove_to_device_events(
    user_id: &UserId,
    device_id: &DeviceId,
    until_sn: Seqnum,
) -> DataResult<()> {
    diesel::delete(
        device_inboxes::table
            .filter(device_inboxes::user_id.eq(user_id))
            .filter(device_inboxes::device_id.eq(device_id))
            .filter(device_inboxes::occur_sn.le(until_sn)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}
