use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::{EmptyResult, JsonResult, MatrixError, data, empty_ok, json_ok, utils};

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDeviceReqBody {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDeviceReqBody {
    pub device_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteDevicesReqBody {
    pub devices: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminDeviceResponse {
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ts: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_user_agent: Option<String>,
    pub user_id: String,
    pub dehydrated: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdminDevicesListResponse {
    pub devices: Vec<AdminDeviceResponse>,
    pub total: i64,
}

pub fn router() -> Router {
    Router::with_path("v2").push(
        Router::with_path("users/{user_id}")
            .push(
                Router::with_path("devices")
                    .get(list_devices)
                    .post(create_device)
                    .push(
                        Router::with_path("{device_id}")
                            .get(get_device)
                            .put(put_device)
                            .delete(delete_device),
                    ),
            )
            .push(Router::with_path("delete_devices").post(delete_devices)),
    )
}

fn to_admin_device_response(device: data::user::device::DbUserDevice) -> AdminDeviceResponse {
    AdminDeviceResponse {
        device_id: device.device_id.to_string(),
        display_name: device.display_name,
        last_seen_ip: device.last_seen_ip,
        last_seen_ts: device.last_seen_at.map(|t| t.get() as i64),
        last_seen_user_agent: device.user_agent,
        user_id: device.user_id.to_string(),
        dehydrated: false,
    }
}

#[endpoint]
pub fn list_devices(user_id: PathParam<OwnedUserId>) -> JsonResult<AdminDevicesListResponse> {
    let user_id = user_id.into_inner();

    let devices = data::user::device::get_devices(&user_id)?;
    let total = devices.len() as i64;
    let devices = devices.into_iter().map(to_admin_device_response).collect();

    json_ok(AdminDevicesListResponse { devices, total })
}

#[endpoint]
pub fn create_device(
    user_id: PathParam<OwnedUserId>,
    body: JsonBody<CreateDeviceReqBody>,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let body = body.into_inner();

    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }

    let Some(device_id_str) = body.device_id else {
        return Err(MatrixError::missing_param("Missing device_id").into());
    };

    let device_id = <OwnedDeviceId>::try_from(device_id_str.as_str())
        .map_err(|_| MatrixError::invalid_param("Invalid device_id"))?;

    if data::user::device::is_device_exists(&user_id, &device_id)? {
        return empty_ok();
    }

    let token = utils::random_string(64);

    data::user::device::create_device(&user_id, &device_id, &token, body.display_name, None)?;

    empty_ok()
}

#[endpoint]
pub fn get_device(
    user_id: PathParam<OwnedUserId>,
    device_id: PathParam<OwnedDeviceId>,
) -> JsonResult<AdminDeviceResponse> {
    let Ok(device) = data::user::device::get_device(&user_id, &device_id) else {
        return Err(MatrixError::not_found("device is not found.").into());
    };
    json_ok(to_admin_device_response(device))
}

#[endpoint]
pub fn put_device(
    user_id: PathParam<OwnedUserId>,
    device_id: PathParam<OwnedDeviceId>,
    body: JsonBody<UpdateDeviceReqBody>,
) -> JsonResult<AdminDeviceResponse> {
    let body = body.into_inner();
    let update = data::user::device::DeviceUpdate {
        display_name: Some(body.display_name),
        user_agent: None,
        last_seen_ip: None,
        last_seen_at: None,
    };
    let Ok(device) = data::user::device::update_device(&user_id, &device_id, update) else {
        return Err(MatrixError::not_found("device is not found.").into());
    };
    json_ok(to_admin_device_response(device))
}

#[endpoint]
pub fn delete_device(
    user_id: PathParam<OwnedUserId>,
    device_id: PathParam<OwnedDeviceId>,
) -> EmptyResult {
    data::user::device::remove_device(&user_id, &device_id)?;
    empty_ok()
}

#[endpoint]
pub fn delete_devices(
    user_id: PathParam<OwnedUserId>,
    body: JsonBody<DeleteDevicesReqBody>,
) -> EmptyResult {
    let user_id = user_id.into_inner();
    let body = body.into_inner();

    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }

    for device_id in body.devices {
        let device_id: OwnedDeviceId = device_id.into();
        let _ = data::user::device::remove_device(&user_id, &device_id);
    }

    empty_ok()
}
