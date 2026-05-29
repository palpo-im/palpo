use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::backup::*;
use crate::core::serde::RawJson;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::key_backup::{self, DbRoomKey, DbRoomKeysVersion};
use crate::{AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, empty_ok, hoops, json_ok};

pub fn authed_router() -> Router {
    Router::with_path("room_keys")
        .hoop(hoops::limit_rate)
        .push(
            Router::with_path("keys")
                .get(get_keys)
                .put(add_keys)
                .delete(delete_keys)
                .push(
                    Router::with_path("{room_id}")
                        .get(get_keys_for_room)
                        .put(add_keys_for_room)
                        .delete(delete_room_keys)
                        .push(
                            Router::with_path("{session_id}")
                                .get(get_session_keys)
                                .put(add_keys_for_session)
                                .delete(delete_session_keys),
                        ),
                ),
        )
        .push(
            Router::with_path("version")
                .get(latest_version)
                .post(create_version)
                .push(
                    Router::with_path("{version}")
                        .get(get_version)
                        .post(update_version)
                        .delete(delete_version),
                ),
        )
}

/// #GET /_matrix/client/r0/room_keys/keys
/// Retrieves all keys from the backup.
#[endpoint]
async fn get_keys(
    _aa: AuthArgs,
    version: QueryParam<i64, true>,
    depot: &mut Depot,
) -> JsonResult<KeysResBody> {
    let authed = depot.authed_info()?;
    let version = version.into_inner();
    let rooms = e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(authed.user_id()))
        .filter(e2e_room_keys::version.eq(version))
        .load::<DbRoomKey>(&mut connect().await?)
        .await?
        .into_iter()
        .map(|rk| {
            let DbRoomKey {
                room_id,
                session_data,
                ..
            } = rk;
            (
                room_id,
                RawJson::<RoomKeyBackup>::from_value(&session_data).unwrap(),
            )
        })
        .collect();
    json_ok(KeysResBody { rooms })
}

/// #GET /_matrix/client/r0/room_keys/keys/{room_id}
/// Retrieves all keys from the backup for a given room.
#[endpoint]
async fn get_keys_for_room(
    _aa: AuthArgs,
    args: KeysForRoomReqArgs,
    depot: &mut Depot,
) -> JsonResult<KeysForRoomResBody> {
    let authed = depot.authed_info()?;
    let DbRoomKey {
        room_id,
        session_data,
        ..
    } = key_backup::get_room_key(authed.user_id(), &args.room_id, args.version)
        .await?
        .ok_or(MatrixError::not_found(
            "Backup key not found for this user's room.",
        ))?;
    json_ok(KeysForRoomResBody::new(BTreeMap::from_iter(
        [(
            room_id,
            RawJson::<RoomKeyBackup>::from_value(&session_data).unwrap(),
        )]
        .into_iter(),
    )))
}
/// #GET /_matrix/client/r0/room_keys/keys/{room_id}/{session_id}
/// Retrieves a key from the backup.
#[endpoint]
async fn get_session_keys(
    _aa: AuthArgs,
    args: KeysForSessionReqArgs,
    depot: &mut Depot,
) -> JsonResult<KeyBackupData> {
    let authed = depot.authed_info()?;
    let key_backup_data = e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(authed.user_id()))
        .filter(e2e_room_keys::version.eq(args.version))
        .filter(e2e_room_keys::room_id.eq(&args.room_id))
        .filter(e2e_room_keys::session_id.eq(&args.session_id))
        // .select(e2e_room_keys::session_data)
        .first::<DbRoomKey>(&mut connect().await?)
        .await
        .optional()?
        .ok_or(MatrixError::not_found(
            "Backup key not found for this user's session.",
        ))?
        .into();

    json_ok(key_backup_data)
}

/// #PUT /_matrix/client/r0/room_keys/keys
/// Add the received backup keys to the database.
///
/// - Only manipulating the most recently created version of the backup is allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
#[endpoint]
async fn add_keys(
    _aa: AuthArgs,
    version: QueryParam<i64, true>,
    body: JsonBody<AddKeysReqBody>,
    depot: &mut Depot,
) -> JsonResult<ModifyKeysResBody> {
    let authed = depot.authed_info()?;
    let version = version.into_inner();

    let keys_version = key_backup::get_latest_room_keys_version(authed.user_id())
        .await?
        .ok_or(MatrixError::not_found("Key backup does not exist."))?;
    if version != keys_version.version {
        return Err(MatrixError::invalid_param(
            "You may only manipulate the most recently created version of the backup.",
        )
        .into());
    }

    for (room_id, room) in &body.rooms {
        for (session_id, key_data) in &room.sessions {
            key_backup::add_key(authed.user_id(), version, room_id, session_id, key_data).await?
        }
    }

    json_ok(ModifyKeysResBody {
        count: (key_backup::count_keys(authed.user_id(), version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), version).await?,
    })
}

/// #PUT /_matrix/client/r0/room_keys/keys/{room_id}
/// Add the received backup keys to the database.
///
/// - Only manipulating the most recently created version of the backup is allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
#[endpoint]
async fn add_keys_for_room(
    _aa: AuthArgs,
    args: KeysForRoomReqArgs,
    body: JsonBody<AddKeysForRoomReqBody>,
    depot: &mut Depot,
) -> JsonResult<ModifyKeysResBody> {
    let authed = depot.authed_info()?;

    let keys_version = key_backup::get_latest_room_keys_version(authed.user_id())
        .await?
        .ok_or(MatrixError::not_found("Key backup does not exist."))?;
    if args.version != keys_version.version {
        return Err(MatrixError::invalid_param(
            "You may only manipulate the most recently created version of the backup.",
        )
        .into());
    }

    for (session_id, key_data) in &body.sessions {
        key_backup::add_key(
            authed.user_id(),
            args.version,
            &args.room_id,
            session_id,
            key_data,
        )
        .await?
    }

    json_ok(ModifyKeysResBody {
        count: (key_backup::count_keys(authed.user_id(), args.version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), args.version).await?,
    })
}
/// #PUT /_matrix/client/r0/room_keys/keys/{room_d}/{session_id}
/// Add the received backup key to the database.
///
/// - Only manipulating the most recently created version of the backup is allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
#[endpoint]
async fn add_keys_for_session(
    _aa: AuthArgs,
    args: KeysForSessionReqArgs,
    body: JsonBody<AddKeysForSessionReqBody>,
    depot: &mut Depot,
) -> JsonResult<KeyBackupData> {
    let authed = depot.authed_info()?;
    let body = body.into_inner();

    let keys_version = key_backup::get_latest_room_keys_version(authed.user_id())
        .await?
        .ok_or(MatrixError::not_found("Key backup does not exist."))?;
    if args.version != keys_version.version {
        return Err(MatrixError::invalid_param(
            "You may only manipulate the most recently created version of the backup.",
        )
        .into());
    }

    key_backup::add_key(
        authed.user_id(),
        args.version,
        &args.room_id,
        &args.session_id,
        &body.0,
    )
    .await?;

    // json_ok(ModifyKeysResBody {
    //     count: (key_backup::count_keys(authed.user_id(), args.version)? as u32).into(),
    //     etag: key_backup::get_etag(authed.user_id(), args.version)?,
    // })
    json_ok(body.0)
}

/// #GET /_matrix/client/r0/room_keys/version/{version}
/// Get information about an existing backup.
#[endpoint]
async fn get_version(
    _aa: AuthArgs,
    version: PathParam<i64>,
    depot: &mut Depot,
) -> JsonResult<VersionResBody> {
    let authed = depot.authed_info()?;
    let version = version.into_inner();
    let algorithm = key_backup::get_room_keys_version(authed.user_id(), version)
        .await?
        .ok_or(MatrixError::not_found("Key backup does not exist."))?
        .algorithm;

    json_ok(VersionResBody {
        algorithm: serde_json::from_value(algorithm)?,
        count: (key_backup::count_keys(authed.user_id(), version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), version).await?,
        version: version.to_string(),
    })
}

/// #POST /_matrix/client/r0/room_keys/version
/// Creates a new backup.
#[endpoint]
async fn create_version(
    _aa: AuthArgs,
    body: JsonBody<CreateVersionReqBody>,
    depot: &mut Depot,
) -> JsonResult<CreateVersionResBody> {
    let authed = depot.authed_info()?;
    let algorithm = body.into_inner().0;
    let version = key_backup::create_backup(authed.user_id(), &algorithm)
        .await?
        .version
        .to_string();

    json_ok(CreateVersionResBody { version })
}

/// #PUT /_matrix/client/r0/room_keys/version/{version}
/// Update information about an existing backup. Only `auth_data` can be modified.
#[endpoint]
async fn update_version(
    _aa: AuthArgs,
    body: JsonBody<CreateVersionReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let algorithm = body.into_inner().0;
    key_backup::create_backup(authed.user_id(), &algorithm).await?;

    empty_ok()
}

/// #GET /_matrix/client/r0/room_keys/version
/// Get information about the latest backup version.
#[endpoint]
async fn latest_version(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<VersionResBody> {
    let authed = depot.authed_info()?;

    let DbRoomKeysVersion {
        version, algorithm, ..
    } = key_backup::get_latest_room_keys_version(authed.user_id())
        .await?
        .ok_or(MatrixError::not_found("Key backup does not exist."))?;

    json_ok(VersionResBody {
        algorithm: serde_json::from_value(algorithm)?,
        count: (key_backup::count_keys(authed.user_id(), version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), version).await?,
        version: version.to_string(),
    })
}
/// #DELETE /_matrix/client/r0/room_keys/version/{version}
/// Delete an existing key backup.
///
/// - Deletes both information about the backup, as well as all key data related to the backup
#[endpoint]
async fn delete_version(_aa: AuthArgs, version: PathParam<i64>, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    let version = version.into_inner();

    key_backup::delete_backup(authed.user_id(), version).await?;

    empty_ok()
}

/// #DELETE /_matrix/client/r0/room_keys/keys
/// Delete the keys from the backup.
#[endpoint]
async fn delete_keys(
    _aa: AuthArgs,
    version: QueryParam<i64, true>,
    depot: &mut Depot,
) -> JsonResult<ModifyKeysResBody> {
    let authed = depot.authed_info()?;
    let version = version.into_inner();
    key_backup::delete_all_keys(authed.user_id(), version).await?;

    json_ok(ModifyKeysResBody {
        count: (key_backup::count_keys(authed.user_id(), version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), version).await?,
    })
}
/// #DELETE /_matrix/client/r0/room_keys/keys/{room_id}
/// Delete the keys from the backup for a given room.
#[endpoint]
async fn delete_room_keys(
    _aa: AuthArgs,
    args: KeysForRoomReqArgs,
    depot: &mut Depot,
) -> JsonResult<ModifyKeysResBody> {
    let authed = depot.authed_info()?;

    key_backup::delete_room_keys(authed.user_id(), args.version, &args.room_id).await?;

    json_ok(ModifyKeysResBody {
        count: (key_backup::count_keys(authed.user_id(), args.version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), args.version).await?,
    })
}
/// #DELETE /_matrix/client/r0/room_keys/keys/{room_id}/{session_id}
/// Delete a key from the backup.
#[endpoint]
async fn delete_session_keys(
    _aa: AuthArgs,
    args: KeysForSessionReqArgs,
    depot: &mut Depot,
) -> JsonResult<ModifyKeysResBody> {
    let authed = depot.authed_info()?;

    key_backup::delete_room_key(
        authed.user_id(),
        args.version,
        &args.room_id,
        &args.session_id,
    )
    .await?;

    json_ok(ModifyKeysResBody {
        count: (key_backup::count_keys(authed.user_id(), args.version).await? as u32).into(),
        etag: key_backup::get_etag(authed.user_id(), args.version).await?,
    })
}
