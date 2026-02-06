//! MAS (Matrix Authentication Service) Modern API Endpoints
//!
//! - GET  _synapse/mas/query_user
//! - POST _synapse/mas/provision_user
//! - GET  _synapse/mas/is_localpart_available
//! - POST _synapse/mas/upsert_device
//! - POST _synapse/mas/update_device_display_name
//! - POST _synapse/mas/delete_device
//! - POST _synapse/mas/sync_devices
//! - POST _synapse/mas/delete_user
//! - POST _synapse/mas/reactivate_user
//! - POST _synapse/mas/set_displayname
//! - POST _synapse/mas/unset_displayname
//! - POST _synapse/mas/allow_cross_signing_reset

use std::collections::HashSet;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::{EmptyResult, JsonResult, MatrixError};
use crate::{config, data, empty_ok, json_ok, user, utils};

fn localpart_to_user_id(localpart: &str) -> crate::AppResult<OwnedUserId> {
    let s = format!("@{}:{}", localpart, config::server_name());
    UserId::parse(&s).map_err(|_| MatrixError::invalid_param("Invalid localpart").into())
}

pub fn router() -> Router {
    Router::new()
        .push(Router::with_path("query_user").get(query_user))
        .push(Router::with_path("provision_user").post(provision_user))
        .push(Router::with_path("is_localpart_available").get(is_localpart_available))
        .push(Router::with_path("upsert_device").post(upsert_device))
        .push(Router::with_path("update_device_display_name").post(update_device_display_name))
        .push(Router::with_path("delete_device").post(mas_delete_device))
        .push(Router::with_path("sync_devices").post(sync_devices))
        .push(Router::with_path("delete_user").post(delete_user))
        .push(Router::with_path("reactivate_user").post(reactivate_user))
        .push(Router::with_path("set_displayname").post(set_displayname))
        .push(Router::with_path("unset_displayname").post(unset_displayname))
        .push(Router::with_path("allow_cross_signing_reset").post(allow_cross_signing_reset))
}

// Types

#[derive(Debug, Serialize, ToSchema)]
pub struct QueryUserResponse {
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub is_deactivated: bool,
    pub is_suspended: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProvisionUserReqBody {
    pub localpart: String,
    #[serde(default)]
    pub set_displayname: Option<String>,
    #[serde(default)]
    pub unset_displayname: bool,
    #[serde(default)]
    pub set_avatar_url: Option<String>,
    #[serde(default)]
    pub unset_avatar_url: bool,
    #[serde(default)]
    pub set_emails: Option<Vec<String>>,
    #[serde(default)]
    pub unset_emails: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertDeviceReqBody {
    pub localpart: String,
    pub device_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDeviceDisplayNameReqBody {
    pub localpart: String,
    pub device_id: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteDeviceReqBody {
    pub localpart: String,
    pub device_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncDevicesReqBody {
    pub localpart: String,
    pub devices: HashSet<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteUserReqBody {
    pub localpart: String,
    #[serde(default)]
    pub erase: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LocalpartReqBody {
    pub localpart: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetDisplaynameReqBody {
    pub localpart: String,
    pub displayname: String,
}

// Endpoints

/// GET _synapse/mas/query_user?localpart=X
#[endpoint]
pub async fn query_user(localpart: QueryParam<String, true>) -> JsonResult<QueryUserResponse> {
    let user_id = localpart_to_user_id(&localpart.into_inner())?;
    let db_user =
        data::user::get_user(&user_id).map_err(|_| MatrixError::not_found("User not found"))?;
    let display_name = data::user::display_name(&user_id).ok().flatten();
    let avatar_url = data::user::avatar_url(&user_id).ok().flatten();
    json_ok(QueryUserResponse {
        user_id: user_id.to_string(),
        display_name,
        avatar_url: avatar_url.map(|u| u.to_string()),
        is_deactivated: db_user.deactivated_at.is_some(),
        is_suspended: db_user.suspended_at.is_some(),
    })
}

/// POST _synapse/mas/provision_user
#[endpoint]
pub async fn provision_user(
    body: JsonBody<ProvisionUserReqBody>,
    res: &mut salvo::Response,
) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    let exists = data::user::user_exists(&user_id)?;
    if !exists {
        user::create_user(user_id.clone(), None)?;
        res.status_code(salvo::http::StatusCode::CREATED);
    }
    if let Some(displayname) = &body.set_displayname {
        data::user::set_display_name(&user_id, displayname)?;
    } else if body.unset_displayname {
        data::user::remove_display_name(&user_id)?;
    }
    if let Some(avatar_url) = &body.set_avatar_url {
        let mxc_uri: &MxcUri = avatar_url.as_str().into();
        data::user::set_avatar_url(&user_id, mxc_uri)?;
    } else if body.unset_avatar_url {
        data::user::remove_avatar_url(&user_id)?;
    }
    if let Some(emails) = body.set_emails {
        let entries: Vec<(String, String, Option<i64>, Option<i64>)> = emails
            .into_iter()
            .map(|email| ("email".to_string(), email, None, None))
            .collect();
        data::user::replace_threepids(&user_id, &entries)?;
    } else if body.unset_emails {
        data::user::replace_threepids(&user_id, &[])?;
    }
    empty_ok()
}

/// GET _synapse/mas/is_localpart_available?localpart=X
#[endpoint]
pub async fn is_localpart_available(localpart: QueryParam<String, true>) -> EmptyResult {
    let localpart = localpart.into_inner();
    if !localpart.is_ascii() {
        return Err(MatrixError::invalid_param("Invalid username").into());
    }
    let user_id = localpart_to_user_id(&localpart)?;
    if data::user::user_exists(&user_id)? {
        return Err(MatrixError::unknown("User ID already taken.").into());
    }
    empty_ok()
}

/// POST _synapse/mas/upsert_device
#[endpoint]
pub async fn upsert_device(body: JsonBody<UpsertDeviceReqBody>) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    let device_id: OwnedDeviceId = body.device_id.into();
    if data::user::device::is_device_exists(&user_id, &device_id)? {
        if let Some(display_name) = body.display_name {
            data::user::device::update_device(
                &user_id, &device_id,
                data::user::device::DeviceUpdate {
                    display_name: Some(Some(display_name)),
                    user_agent: None, last_seen_ip: None, last_seen_at: None,
                },
            )?;
        }
    } else {
        let token = utils::random_string(64);
        data::user::device::create_device(&user_id, &device_id, &token, body.display_name, None)?;
    }
    empty_ok()
}

/// POST _synapse/mas/update_device_display_name
#[endpoint]
pub async fn update_device_display_name(
    body: JsonBody<UpdateDeviceDisplayNameReqBody>,
) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    let device_id: OwnedDeviceId = body.device_id.into();
    data::user::device::update_device(
        &user_id, &device_id,
        data::user::device::DeviceUpdate {
            display_name: Some(Some(body.display_name)),
            user_agent: None, last_seen_ip: None, last_seen_at: None,
        },
    )?;
    empty_ok()
}

/// POST _synapse/mas/delete_device
#[endpoint]
pub async fn mas_delete_device(body: JsonBody<DeleteDeviceReqBody>) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    let device_id: OwnedDeviceId = body.device_id.into();
    data::user::device::remove_device(&user_id, &device_id)?;
    empty_ok()
}

/// POST _synapse/mas/sync_devices
#[endpoint]
pub async fn sync_devices(body: JsonBody<SyncDevicesReqBody>) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    let current_devices = data::user::device::get_devices(&user_id)?;
    let current_ids: HashSet<String> =
        current_devices.iter().map(|d| d.device_id.to_string()).collect();
    for device in &current_devices {
        if !body.devices.contains(device.device_id.as_str()) {
            let _ = data::user::device::remove_device(&user_id, &device.device_id);
        }
    }
    for device_id_str in &body.devices {
        if !current_ids.contains(device_id_str) {
            let device_id: OwnedDeviceId = device_id_str.as_str().into();
            let token = utils::random_string(64);
            data::user::device::create_device(&user_id, &device_id, &token, None, None)?;
        }
    }
    empty_ok()
}

/// POST _synapse/mas/delete_user
#[endpoint]
pub async fn delete_user(body: JsonBody<DeleteUserReqBody>) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }
    let joined_rooms = data::user::joined_rooms(&user_id)?;
    user::full_user_deactivate(&user_id, &joined_rooms).await?;
    if body.erase {
        user::delete_all_media(&user_id).await?;
    }
    empty_ok()
}

/// POST _synapse/mas/reactivate_user
#[endpoint]
pub async fn reactivate_user(body: JsonBody<LocalpartReqBody>) -> EmptyResult {
    let user_id = localpart_to_user_id(&body.into_inner().localpart)?;
    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }
    data::user::reactivate(&user_id)?;
    empty_ok()
}

/// POST _synapse/mas/set_displayname
#[endpoint]
pub async fn set_displayname(body: JsonBody<SetDisplaynameReqBody>) -> EmptyResult {
    let body = body.into_inner();
    let user_id = localpart_to_user_id(&body.localpart)?;
    data::user::set_display_name(&user_id, &body.displayname)?;
    empty_ok()
}

/// POST _synapse/mas/unset_displayname
#[endpoint]
pub async fn unset_displayname(body: JsonBody<LocalpartReqBody>) -> EmptyResult {
    let user_id = localpart_to_user_id(&body.into_inner().localpart)?;
    data::user::remove_display_name(&user_id)?;
    empty_ok()
}

/// POST _synapse/mas/allow_cross_signing_reset
#[endpoint]
pub async fn allow_cross_signing_reset(body: JsonBody<LocalpartReqBody>) -> EmptyResult {
    let user_id = localpart_to_user_id(&body.into_inner().localpart)?;
    if !data::user::user_exists(&user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }
    if !data::user::key::has_master_cross_signing_key(&user_id)? {
        return Err(MatrixError::not_found("User has no master cross-signing key").into());
    }
    let now_ms = crate::core::UnixMillis::now().get() as i64;
    let expires_ts = now_ms + 10 * 60 * 1000;
    data::user::key::set_cross_signing_replacement_allowed(&user_id, expires_ts)?;
    empty_ok()
}
