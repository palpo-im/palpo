//! Endpoints for handling keys for end-to-end encryption

use diesel::prelude::*;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::AuthArgs;
use crate::core::federation::device::{Device, DevicesResBody};
use crate::core::federation::key::{ClaimKeysReqBody, ClaimKeysResBody, KeysReqBody, KeysResBody};
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{AppError, CjsonResult, JsonResult, cjson_ok, db, json_ok};

pub fn router() -> Router {
    Router::with_path("user")
        .push(
            Router::with_path("keys")
                .push(Router::with_path("claim").post(claim_keys))
                .push(Router::with_path("query").post(query_keys)),
        )
        .push(Router::with_path("devices/{user_id}").get(get_devices))
}

/// #POST /_matrix/federation/v1/user/keys/claim
/// Claims one-time keys.
#[endpoint]
async fn claim_keys(_aa: AuthArgs, body: JsonBody<ClaimKeysReqBody>) -> CjsonResult<ClaimKeysResBody> {
    let result = crate::user::claim_one_time_keys(&body.one_time_keys).await?;

    cjson_ok(ClaimKeysResBody {
        one_time_keys: result.one_time_keys,
    })
}
/// #POST /_matrix/federation/v1/user/keys/query
/// Gets devices and identity keys for the given users.
#[endpoint]
async fn query_keys(_aa: AuthArgs, body: JsonBody<KeysReqBody>) -> CjsonResult<KeysResBody> {
    let server_name = &crate::config().server_name;
    let result = crate::user::query_keys(None, &body.device_keys, |u| u.server_name() == server_name).await?;

    cjson_ok(KeysResBody {
        device_keys: result.device_keys,
        master_keys: result.master_keys,
        self_signing_keys: result.self_signing_keys,
    })
}

/// #GET /_matrix/federation/v1/user/devices/{user_id}
/// Gets information on all devices of the user.
#[endpoint]
fn get_devices(_aa: AuthArgs, user_id: PathParam<OwnedUserId>) -> JsonResult<DevicesResBody> {
    let server_name = &crate::config().server_name;
    let user_id = user_id.into_inner();
    let stream_id = device_streams::table
        .filter(device_streams::user_id.eq(&user_id))
        .select(device_streams::id)
        .order_by(device_streams::id.desc())
        .first::<i64>(&mut *db::connect()?)
        .optional()?
        .unwrap_or_default();

    let mut devices = vec![];
    for (device_id, display_name) in user_devices::table
        .filter(user_devices::user_id.eq(&user_id))
        .select((user_devices::device_id, user_devices::display_name))
        .load::<(OwnedDeviceId, Option<String>)>(&mut *db::connect()?)?
    {
        devices.push(Device {
            keys: crate::user::get_device_keys_and_sigs(&user_id, &device_id)?
                .ok_or_else(|| AppError::public("server keys not found"))?,
            device_id,
            device_display_name: display_name,
        })
    }
    json_ok(DevicesResBody {
        stream_id: stream_id as u64,
        devices,
        master_key: crate::user::get_master_key(Some(&user_id), &user_id, &|u| u.server_name() == server_name)?,
        self_signing_key: crate::user::get_self_signing_key(Some(&user_id), &user_id, &|u| {
            u.server_name() == server_name
        })?,
        user_id: user_id.to_owned(),
    })
}
