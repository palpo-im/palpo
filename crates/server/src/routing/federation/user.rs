//! Endpoints for handling keys for end-to-end encryption

use diesel::prelude::*;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::device::{Device, DevicesResBody};
use crate::core::federation::key::{
    ClaimKeysReqBody, ClaimKeysResBody, QueryKeysReqBody, QueryKeysResBody,
};
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppError, AuthArgs, CjsonResult, DepotExt, JsonResult, cjson_ok, data, json_ok};

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
async fn claim_keys(
    _aa: AuthArgs,
    body: JsonBody<ClaimKeysReqBody>,
) -> CjsonResult<ClaimKeysResBody> {
    let result = crate::user::claim_one_time_keys(&body.one_time_keys).await?;

    cjson_ok(ClaimKeysResBody {
        one_time_keys: result.one_time_keys,
    })
}
/// #POST /_matrix/federation/v1/user/keys/query
/// Gets devices and identity keys for the given users.
#[endpoint]
async fn query_keys(
    _aa: AuthArgs,
    body: JsonBody<QueryKeysReqBody>,
    depot: &mut Depot,
) -> CjsonResult<QueryKeysResBody> {
    let origin = depot.origin()?;
    let result = crate::user::query_keys(
        None,
        &body.device_keys,
        |u| u.server_name() == origin,
        false,
    )
    .await?;

    cjson_ok(QueryKeysResBody {
        device_keys: result.device_keys,
        master_keys: result.master_keys,
        self_signing_keys: result.self_signing_keys,
    })
}

/// #GET /_matrix/federation/v1/user/devices/{user_id}
/// Gets information on all devices of the user.
#[endpoint]
fn get_devices(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
) -> JsonResult<DevicesResBody> {
    let origin = depot.origin()?;
    let user_id = user_id.into_inner();
    let stream_id = device_streams::table
        .filter(device_streams::user_id.eq(&user_id))
        .select(device_streams::id)
        .order_by(device_streams::id.desc())
        .first::<i64>(&mut connect()?)
        .optional()?
        .unwrap_or_default();

    let mut devices = vec![];
    let devices_and_names = user_devices::table
        .filter(user_devices::user_id.eq(&user_id))
        .select((user_devices::device_id, user_devices::display_name))
        .load::<(OwnedDeviceId, Option<String>)>(&mut connect()?)?;
    for (device_id, display_name) in devices_and_names {
        devices.push(Device {
            keys: data::user::get_device_keys_and_sigs(&user_id, &device_id)?
                .ok_or_else(|| AppError::public("server keys not found"))?,
            device_id,
            device_display_name: display_name,
        })
    }
    json_ok(DevicesResBody {
        stream_id: stream_id as u64,
        devices,
        master_key: crate::user::get_allowed_master_key(Some(&user_id), &user_id, &|u| {
            u.server_name() == origin
        })?,
        self_signing_key: crate::user::get_allowed_self_signing_key(
            Some(&user_id),
            &user_id,
            &|u| u.server_name() == origin,
        )?,
        user_id: user_id.to_owned(),
    })
}
