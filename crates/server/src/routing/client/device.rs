use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use palpo_core::UnixMillis;
use salvo::oapi::extract::{JsonBody, PathParam};
use salvo::prelude::*;

use crate::core::OwnedDeviceId;
use crate::core::client::dehydrated_device::{
    DeleteDehydratedDeviceResBody, GetDehydratedDeviceResBody, UpsertDehydratedDeviceReqBody,
    UpsertDehydratedDeviceResBody,
};
use crate::core::client::device::{
    DeleteDeviceReqBody, DeleteDevicesReqBody, DeviceResBody, DevicesResBody, UpdatedDeviceReqBody,
};
use crate::core::client::uiaa::{AuthError, AuthFlow, AuthType, UiaaInfo};
use crate::core::error::ErrorKind;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::DbUserDevice;
use crate::{
    AppError, AuthArgs, DEVICE_ID_LENGTH, DepotExt, EmptyResult, JsonResult, MatrixError, data,
    empty_ok, json_ok, utils,
};

pub fn authed_router() -> Router {
    Router::new()
        .push(
            Router::with_path("devices")
                .get(list_devices)
                // .push(
                //     Router::with_hoop(hoops::limit_rate)
                //         .push(Router::new().post(register).push(Router::with_path("available").
                // get(available)))         .push(Router::with_path("m.login.
                // registration_token/validity").get(validate_token)), )
                .push(
                    Router::with_path("{device_id}")
                        .get(get_device)
                        .delete(delete_device)
                        .put(update_device),
                ),
        )
        // `/delete_devices` is a sibling of `/devices`, not a child of it.
        .push(Router::with_path("delete_devices").post(delete_devices))
}

/// #GET /_matrix/client/r0/devices/{device_id}
/// Get metadata on a single device of the sender user.
#[endpoint]
async fn get_device(
    _aa: AuthArgs,
    device_id: PathParam<OwnedDeviceId>,
    depot: &mut Depot,
) -> JsonResult<DeviceResBody> {
    let authed = depot.authed_info()?;

    let Ok(device) = data::user::device::get_device(authed.user_id(), &device_id).await else {
        return Err(MatrixError::not_found("Device is not found.").into());
    };
    json_ok(DeviceResBody(device.into_matrix_device()))
}

/// #GET /_matrix/client/r0/devices
/// Get metadata on all devices of the sender user.
#[endpoint]
async fn list_devices(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<DevicesResBody> {
    let authed = depot.authed_info()?;

    let devices = user_devices::table
        .filter(user_devices::user_id.eq(authed.user_id()))
        .load::<DbUserDevice>(&mut connect().await?)
        .await?;
    json_ok(DevicesResBody {
        devices: devices
            .into_iter()
            .map(DbUserDevice::into_matrix_device)
            .collect(),
    })
}

/// #PUT /_matrix/client/r0/devices/{device_id}
/// Updates the metadata on a given device of the sender user.
#[endpoint]
async fn update_device(
    _aa: AuthArgs,
    device_id: PathParam<OwnedDeviceId>,
    body: JsonBody<UpdatedDeviceReqBody>,
    req: &mut Request,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let device_id = device_id.into_inner();
    let device = update_owned_device_metadata(
        authed.user_id(),
        &device_id,
        body.display_name.as_deref(),
        &req.remote_addr().to_string(),
    )
    .await?;

    if let Some(device) = device {
        crate::user::key::send_device_key_update(&device.user_id, &device_id).await?;
    } else {
        let Some(appservice) = authed.appservice() else {
            return Err(MatrixError::not_found("Device is not found.").into());
        };
        if !appservice.registration.device_management {
            return Err(MatrixError::not_found("Device is not found.").into());
        }
        debug!(
            "Creating new device for {} from appservice {} as MSC4190 is enabled and device ID does not exist",
            authed.user_id(),
            appservice.registration.id
        );

        let device_id = OwnedDeviceId::from(utils::random_string(DEVICE_ID_LENGTH));

        let device = data::user::device::create_device(
            authed.user_id(),
            &device_id,
            &appservice.registration.as_token,
            None,
            Some(req.remote_addr().to_string()),
        )
        .await?;
        crate::user::key::send_device_key_update(&device.user_id, &device_id).await?;
    }

    empty_ok()
}

/// Scope the mutation itself to the owner, including when device IDs collide.
async fn update_owned_device_metadata(
    user_id: &crate::core::UserId,
    device_id: &crate::core::DeviceId,
    display_name: Option<&str>,
    remote_ip: &str,
) -> crate::AppResult<Option<DbUserDevice>> {
    Ok(diesel::update(
        user_devices::table
            .filter(user_devices::user_id.eq(user_id))
            .filter(user_devices::device_id.eq(device_id)),
    )
    .set((
        user_devices::display_name.eq(display_name),
        user_devices::last_seen_ip.eq(remote_ip),
        user_devices::last_seen_at.eq(UnixMillis::now()),
    ))
    .get_result::<DbUserDevice>(&mut connect().await?)
    .await
    .optional()?)
}

/// #DELETE /_matrix/client/r0/devices/{deviceId}
/// Deletes the given device.
///
/// - Requires UIAA to verify user password
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
#[endpoint]
async fn delete_device(
    _aa: AuthArgs,
    device_id: PathParam<OwnedDeviceId>,
    body: JsonBody<Option<DeleteDeviceReqBody>>,
    depot: &mut Depot,
    res: &mut Response,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let auth = body.into_inner().and_then(|body| body.auth);
    let device_id = device_id.into_inner();

    // UIAA
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let Some(auth) = auth else {
        uiaa_info.auth_error = Some(AuthError::new(
            ErrorKind::Unauthorized,
            "Missing authentication data",
        ));
        crate::uiaa::create_challenge_session(
            authed.user_id(),
            authed.require_device_id()?,
            &mut uiaa_info,
        )
        .await?;
        return Err(uiaa_info.into());
    };

    let (authenticated, uiaa) = match crate::uiaa::try_auth(
        authed.user_id(),
        authed.require_device_id()?,
        &auth,
        &uiaa_info,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            if let AppError::Matrix(e) = e
                && let ErrorKind::Forbidden = e.kind
            {
                return Err(e.into());
            }
            uiaa_info.auth_error = Some(AuthError::new(
                ErrorKind::Forbidden,
                "Invalid authentication data",
            ));
            crate::uiaa::create_challenge_session(
                authed.user_id(),
                authed.require_device_id()?,
                &mut uiaa_info,
            )
            .await?;
            res.status_code(StatusCode::UNAUTHORIZED); // TestDeviceManagement asks http code 401
            return Err(uiaa_info.into());
        }
    };
    if !authenticated {
        res.status_code(StatusCode::UNAUTHORIZED); // TestDeviceManagement asks http code 401
        return Err(uiaa.into());
    }
    crate::user::remove_device(authed.user_id(), &device_id).await?;
    empty_ok()
}

/// #POST /_matrix/client/r0/delete_devices
/// Deletes the given list of devices.
///
/// - Requires UIAA to verify user password
///
/// For each device:
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
#[endpoint]
async fn delete_devices(
    _aa: AuthArgs,
    body: JsonBody<DeleteDevicesReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let DeleteDevicesReqBody { devices, auth } = body.into_inner();

    // UIAA
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let Some(auth) = auth else {
        crate::uiaa::create_challenge_session(
            authed.user_id(),
            authed.require_device_id()?,
            &mut uiaa_info,
        )
        .await?;
        return Err(uiaa_info.into());
    };

    let (authenticated, uiaa) = crate::uiaa::try_auth(
        authed.user_id(),
        authed.require_device_id()?,
        &auth,
        &uiaa_info,
    )
    .await?;
    if !authenticated {
        return Err(uiaa.into());
    }
    for device_id in devices {
        crate::user::remove_device(authed.user_id(), &device_id).await?;
    }

    empty_ok()
}

#[endpoint]
pub(super) async fn dehydrated(
    _aa: AuthArgs,
    depot: &mut Depot,
) -> JsonResult<GetDehydratedDeviceResBody> {
    let authed = depot.authed_info()?;
    let Some((device_id, device_data)) =
        data::user::get_dehydrated_device(authed.user_id()).await?
    else {
        return Err(MatrixError::not_found("No dehydrated device available.").into());
    };

    json_ok(GetDehydratedDeviceResBody::new(device_id, device_data))
}

#[endpoint]
pub(super) async fn delete_dehydrated(
    _aa: AuthArgs,
    depot: &mut Depot,
) -> JsonResult<DeleteDehydratedDeviceResBody> {
    let authed = depot.authed_info()?;
    let Some((device_id, _)) = data::user::get_dehydrated_device(authed.user_id()).await? else {
        return Err(MatrixError::not_found("No dehydrated device available.").into());
    };

    data::user::delete_dehydrated_devices(authed.user_id()).await?;

    crate::user::key::mark_device_key_update(authed.user_id(), &device_id).await?;
    crate::user::key::send_device_key_update(authed.user_id(), &device_id).await?;

    json_ok(DeleteDehydratedDeviceResBody::new(device_id))
}

#[endpoint]
pub(super) async fn upsert_dehydrated(
    _aa: AuthArgs,
    body: JsonBody<UpsertDehydratedDeviceReqBody>,
    depot: &mut Depot,
) -> JsonResult<UpsertDehydratedDeviceResBody> {
    let authed = depot.authed_info()?;
    let UpsertDehydratedDeviceReqBody {
        device_id,
        device_data,
        mut device_keys,
        initial_device_display_name,
        one_time_keys,
        fallback_keys,
    } = body.into_inner();

    if device_keys.user_id != authed.user_id() || device_keys.device_id != device_id {
        return Err(MatrixError::invalid_param(
            "Dehydrated device keys must match the authenticated user and device ID.",
        )
        .into());
    }

    if data::user::device::is_device_exists(authed.user_id(), &device_id).await? {
        return Err(MatrixError::invalid_param(
            "Dehydrated device ID must not match an existing device.",
        )
        .into());
    }

    // Carry the client-supplied display name on the device keys so the dehydrated
    // device is not advertised nameless through `/keys/query`.
    if let Some(display_name) = initial_device_display_name {
        device_keys.unsigned.device_display_name = Some(display_name);
    }

    data::user::upsert_dehydrated_device(authed.user_id(), &device_id, &device_data).await?;

    // Persist the pre-keys before publishing the device. `add_device_keys` sends the
    // device-list update, so storing the one-time/fallback keys first ensures a peer
    // reacting to that update can immediately claim keys for the new device.
    for (key_id, one_time_key) in &one_time_keys {
        crate::user::add_one_time_key(authed.user_id(), &device_id, key_id, one_time_key).await?;
    }

    for (key_id, fallback_key) in &fallback_keys {
        crate::user::add_fallback_key(authed.user_id(), &device_id, key_id, fallback_key).await?;
    }

    crate::user::add_device_keys(authed.user_id(), &device_id, &device_keys).await?;

    json_ok(UpsertDehydratedDeviceResBody::new(device_id))
}

#[cfg(test)]
mod device_update_ownership_tests {
    use super::*;
    use crate::core::UserId;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_device_metadata_updates_are_scoped_to_the_owner() {
        crate::test_database::init();
        let alice = UserId::parse("@q08alice:ownership.example").unwrap();
        let bob = UserId::parse("@q08bob:ownership.example").unwrap();
        for owner in [&alice, &bob] {
            data::user::create_user(&data::user::NewDbUser {
                id: owner.clone(),
                localpart: owner.localpart().into(),
                server_name: owner.server_name().to_owned(),
                is_local: true,
                ty: None,
                is_admin: false,
                is_guest: false,
                appservice_id: None,
                created_at: UnixMillis::now(),
            })
            .await
            .unwrap();
        }
        let shared: OwnedDeviceId = "SHARED".into();
        let bob_only: OwnedDeviceId = "BOB_ONLY".into();
        for (owner, device_id) in [(&alice, &shared), (&bob, &shared), (&bob, &bob_only)] {
            let token = format!("q08-{}-{device_id}", owner.localpart());
            data::user::device::create_device(
                owner,
                device_id,
                &token,
                Some("original".into()),
                Some("original-ip".into()),
            )
            .await
            .unwrap();
        }
        let changed = update_owned_device_metadata(&alice, &shared, Some("alice-new"), "alice-ip")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(changed.user_id, alice);
        assert!(
            update_owned_device_metadata(&alice, &bob_only, Some("unauthorized"), "wrong-ip")
                .await
                .unwrap()
                .is_none()
        );
        for device_id in [&shared, &bob_only] {
            let unchanged = data::user::device::get_device(&bob, device_id)
                .await
                .unwrap();
            assert_eq!(unchanged.display_name.as_deref(), Some("original"));
            assert_eq!(unchanged.last_seen_ip.as_deref(), Some("original-ip"));
        }
        let alice_device = data::user::device::get_device(&alice, &shared)
            .await
            .unwrap();
        assert_eq!(alice_device.display_name.as_deref(), Some("alice-new"));
    }
}
