use std::collections::BTreeMap;

use salvo::oapi::extract::PathParam;
use salvo::prelude::*;

use crate::core::client::server::{ConnectionInfo, DeviceInfo, SessionInfo, UserInfoResBody};
use crate::core::identifiers::*;
use crate::{data, AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

pub fn authed_router() -> Router {
    Router::with_path("admin/whois/{user_id}").get(whois)
}

/// Get information about a particular user.
///
/// This endpoint requires the user to be a server admin, or to be requesting
/// information about themselves.
#[endpoint]
async fn whois(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
) -> JsonResult<UserInfoResBody> {
    let authed = depot.authed_info()?;
    let target_user_id = user_id.into_inner();

    // Check if the user is an admin or requesting their own info
    let is_admin = data::user::is_admin(authed.user_id()).unwrap_or(false);
    if !is_admin && authed.user_id() != &target_user_id {
        return Err(MatrixError::forbidden(
            "Only admins can query other users' information.",
            None,
        )
        .into());
    }

    // Verify user exists
    if !data::user::user_exists(&target_user_id)? {
        return Err(MatrixError::not_found("User not found").into());
    }

    // Get user's devices and session info
    let user_devices = data::user::device::get_devices(&target_user_id)?;
    let mut devices = BTreeMap::new();

    for device in user_devices {
        let connection = ConnectionInfo {
            ip: device.last_seen_ip,
            last_seen: device.last_seen_at,
            user_agent: device.user_agent,
        };

        let session = SessionInfo {
            connections: vec![connection],
        };

        devices.insert(
            device.device_id.to_string(),
            DeviceInfo {
                sessions: vec![session],
            },
        );
    }

    json_ok(UserInfoResBody {
        user_id: Some(target_user_id),
        devices,
    })
}
