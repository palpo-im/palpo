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
    // An existing backup with no sessions for this room is a successful empty
    // read, not a missing backup. In particular, invites to a fresh encrypted
    // room must not require sending a first message just to create a session.
    key_backup::get_room_keys_version(authed.user_id(), args.version)
        .await?
        .ok_or(MatrixError::not_found("Unknown backup version"))?;
    let sessions = e2e_room_keys::table
        .filter(e2e_room_keys::user_id.eq(authed.user_id()))
        .filter(e2e_room_keys::version.eq(args.version))
        .filter(e2e_room_keys::room_id.eq(&args.room_id))
        .load::<DbRoomKey>(&mut connect().await?)
        .await?
        .into_iter()
        .map(|key| (key.session_id.clone(), key.into()))
        .collect();
    json_ok(KeysForRoomResBody::new(sessions))
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
///
/// The version named in the path is updated in place; unlike `create_version`
/// this must not mint a new version, otherwise clients that just signed the
/// backup find their signature on a now-superseded version and never settle on
/// a trusted backup.
#[endpoint]
async fn update_version(
    _aa: AuthArgs,
    version: PathParam<i64>,
    body: JsonBody<CreateVersionReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let version = version.into_inner();
    let algorithm = body.into_inner().0.deserialize()?;

    if !key_backup::update_backup(authed.user_id(), version, &algorithm).await? {
        return Err(MatrixError::not_found("Key backup does not exist.").into());
    }

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

#[cfg(test)]
mod tests {
    use salvo::http::StatusCode;
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;
    use crate::core::UnixMillis;
    use crate::data::user::NewDbUser;

    async fn account(user: &crate::core::UserId, token: &str) {
        crate::data::user::create_user(&NewDbUser {
            id: user.to_owned(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user.localpart().to_owned(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        let device: crate::core::OwnedDeviceId = "BACKUP_TEST".into();
        crate::data::user::device::create_device(user, &device, token, None, None)
            .await
            .unwrap();
    }

    async fn read(
        service: &Service,
        path: &str,
        token: Option<&str>,
    ) -> (Option<StatusCode>, Value) {
        let mut request = TestClient::get(format!("http://localhost/room_keys/{path}"));
        if let Some(token) = token {
            request = request.add_header("authorization", format!("Bearer {token}"), true);
        }
        let mut response = request.send(service).await;
        (
            response.status_code,
            response.take_json::<Value>().await.unwrap(),
        )
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_room_backup_distinguishes_empty_sessions_from_missing_versions() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"backup.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let local = crate::config::get().server_name.to_string();
        let alice = crate::core::UserId::parse(format!("@roombackupalice:{local}")).unwrap();
        let bob = crate::core::UserId::parse(format!("@roombackupbob:{local}")).unwrap();
        account(&alice, "room-backup-alice").await;
        account(&bob, "room-backup-bob").await;
        let algorithm = RawJson::from_value(&json!({
            "algorithm":"m.megolm_backup.v1.curve25519-aes-sha2",
            "auth_data":{"public_key":"Zml4dHVyZQ","signatures":{}}
        }))
        .unwrap();
        let backup = key_backup::create_backup(&alice, &algorithm).await.unwrap();
        let service = Service::new(
            Router::new()
                .hoop(hoops::auth_by_access_token)
                .push(authed_router()),
        );
        let room = crate::core::room_id!("!empty-backup:example.test");
        let path = format!("keys/{room}?version={}", backup.version);
        assert_eq!(
            read(&service, &path, None).await.0,
            Some(StatusCode::UNAUTHORIZED)
        );
        let (status, body) = read(&service, &path, Some("room-backup-alice")).await;
        assert_eq!(status, Some(StatusCode::OK), "{body}");
        assert_eq!(body, json!({"sessions":{}}));
        // A valid backup belonging to somebody else is still missing for Bob.
        let (status, body) = read(&service, &path, Some("room-backup-bob")).await;
        assert_eq!(status, Some(StatusCode::NOT_FOUND));
        assert_eq!(body["errcode"], "M_NOT_FOUND");
        assert_eq!(
            read(
                &service,
                &format!("keys/{room}?version=1"),
                Some("room-backup-alice")
            )
            .await
            .0,
            Some(StatusCode::NOT_FOUND)
        );
        let data: KeyBackupData = serde_json::from_value(json!({
            "first_message_index":7, "forwarded_count":2, "is_verified":true,
            "session_data":{"ciphertext":"fixture-ciphertext","ephemeral":"fixture-ephemeral","mac":"fixture-mac"}
        })).unwrap();
        for session in ["first-session", "second-session"] {
            key_backup::add_key(&alice, backup.version, room, &session.to_owned(), &data)
                .await
                .unwrap();
        }
        key_backup::add_key(
            &alice,
            backup.version,
            crate::core::room_id!("!other-backup:example.test"),
            &"other-room-session".to_owned(),
            &data,
        )
        .await
        .unwrap();
        let (status, body) = read(&service, &path, Some("room-backup-alice")).await;
        assert_eq!(status, Some(StatusCode::OK));
        assert_eq!(
            body,
            json!({"sessions":{"first-session":data,"second-session":data}})
        );
        assert_eq!(
            read(
                &service,
                &format!("keys/{room}/missing-session?version={}", backup.version),
                Some("room-backup-alice")
            )
            .await
            .0,
            Some(StatusCode::NOT_FOUND)
        );
        key_backup::delete_backup(&alice, backup.version)
            .await
            .unwrap();
        let (status, body) = read(&service, &path, Some("room-backup-alice")).await;
        assert_eq!(status, Some(StatusCode::NOT_FOUND));
        assert_eq!(body["errcode"], "M_NOT_FOUND");
    }
}
