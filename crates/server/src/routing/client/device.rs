use diesel::prelude::*;
use palpo_core::UnixMillis;
use salvo::oapi::extract::{JsonBody, PathParam};
use salvo::prelude::*;

use crate::core::OwnedDeviceId;
use crate::core::client::device::{
    DeleteDeviceReqBody, DeleteDevicesReqBody, DeviceResBody, DevicesResBody, UpdatedDeviceReqBody,
};
use crate::core::client::uiaa::{AuthError, AuthFlow, AuthType, UiaaInfo};
use crate::core::error::ErrorKind;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::DbUserDevice;
use crate::{
    AppError, AuthArgs, DEVICE_ID_LENGTH, DepotExt, EmptyResult, JsonResult, MatrixError,
    SESSION_ID_LENGTH, data, empty_ok, json_ok, utils,
};

pub fn authed_router() -> Router {
    Router::with_path("devices")
        .get(list_devices)
        // .push(
        //     Router::with_hoop(hoops::limit_rate)
        //         .push(Router::new().post(register).push(Router::with_path("available").get(available)))
        //         .push(Router::with_path("m.login.registration_token/validity").get(validate_token)),
        // )
        .push(Router::with_path("delete_devices").post(delete_devices))
        .push(
            Router::with_path("{device_id}")
                .get(get_device)
                .delete(delete_device)
                .put(update_device),
        )
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

    let Ok(device) = data::user::device::get_device(authed.user_id(), &device_id) else {
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
        .load::<DbUserDevice>(&mut connect()?)?;
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
fn update_device(
    _aa: AuthArgs,
    device_id: PathParam<OwnedDeviceId>,
    body: JsonBody<UpdatedDeviceReqBody>,
    req: &mut Request,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let device_id = device_id.into_inner();
    let device = user_devices::table
        .filter(user_devices::device_id.eq(&device_id))
        .first::<DbUserDevice>(&mut connect()?)
        .optional()?;

    if let Some(device) = device {
        diesel::update(&device)
            .set((
                user_devices::display_name.eq(&body.display_name),
                user_devices::last_seen_ip.eq(&req.remote_addr().to_string()),
                user_devices::last_seen_at.eq(UnixMillis::now()),
            ))
            .execute(&mut connect()?)?;
        crate::user::key::send_device_key_update(&device.user_id, &device_id)?;
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
        )?;
        crate::user::key::send_device_key_update(&device.user_id, &device_id)?;
    }

    empty_ok()
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
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        uiaa_info.auth_error = Some(AuthError::new(
            ErrorKind::Unauthorized,
            "Missing authentication data",
        ));
        return Err(uiaa_info.into());
    };

    if let Err(e) = crate::uiaa::try_auth(authed.user_id(), authed.device_id(), &auth, &uiaa_info) {
        if let AppError::Matrix(e) = e
            && let ErrorKind::Forbidden { .. } = e.kind
        {
            return Err(e.into());
        }
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        uiaa_info.auth_error = Some(AuthError::new(
            ErrorKind::forbidden(),
            "Invalid authentication data",
        ));
        res.status_code(StatusCode::UNAUTHORIZED); // TestDeviceManagement asks http code 401
        return Err(uiaa_info.into());
    }
    data::user::device::remove_device(authed.user_id(), &device_id)?;
    empty_ok()
}

/// #DELETE /_matrix/client/r0/devices/{deviceId}
/// Deletes the given device.
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
    let uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let Some(auth) = auth else {
        return Err(uiaa_info.into());
    };

    crate::uiaa::try_auth(authed.user_id(), authed.device_id(), &auth, &uiaa_info)?;
    diesel::delete(
        user_devices::table
            .filter(user_devices::user_id.eq(authed.device_id()))
            .filter(user_devices::device_id.eq_any(&devices)),
    )
    .execute(&mut connect()?)?;

    empty_ok()
}

#[endpoint]
pub(super) async fn dehydrated(_aa: AuthArgs) -> EmptyResult {
    // TODO: Later
    empty_ok()
}

#[endpoint]
pub(super) async fn delete_dehydrated(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    data::user::delete_dehydrated_devices(authed.user_id())?;
    empty_ok()
}

#[endpoint]
pub(super) async fn upsert_dehydrated(_aa: AuthArgs) -> EmptyResult {
    // TODO: Later
    empty_ok()
}
