use diesel::prelude::*;
use horme::RunQueryDsl;
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
use crate::data::schema::*;
use crate::data::user::DbUserDevice;
use crate::data::user::device::update_device_metadata_if_present;
use crate::{
    AppError, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, data, empty_ok, json_ok,
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
        .load::<DbUserDevice>(&mut crate::data::db())
        .await?;
    json_ok(DevicesResBody {
        devices: devices
            .into_iter()
            .map(DbUserDevice::into_matrix_device)
            .collect(),
    })
}

/// #PUT /_matrix/client/r0/devices/{device_id}
/// Updates device metadata, or creates the requested device for an authorized Appservice.
#[endpoint]
async fn update_device(
    _aa: AuthArgs,
    device_id: PathParam<OwnedDeviceId>,
    body: JsonBody<UpdatedDeviceReqBody>,
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let device_id = device_id.into_inner();
    let remote_ip = req.remote_addr().to_string();
    let device = update_device_metadata_if_present(
        authed.user_id(),
        &device_id,
        body.display_name.as_deref(),
        &remote_ip,
    )
    .await?;

    let (device, created) = if let Some(device) = device {
        (device, false)
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

        // Appservice requests keep using the scoped service token. Never
        // publish it as an ordinary user token while provisioning a device.
        match data::user::device::insert_device_metadata_if_missing(
            authed.user_id(),
            &device_id,
            body.display_name.clone(),
            Some(remote_ip.clone()),
        )
        .await?
        {
            Some(device) => (device, true),
            None => {
                // A concurrent PUT created this exact scoped device first.
                let device = update_device_metadata_if_present(
                    authed.user_id(),
                    &device_id,
                    body.display_name.as_deref(),
                    &remote_ip,
                )
                .await?
                .ok_or_else(|| MatrixError::not_found("Device is not found."))?;
                (device, false)
            }
        }
    };
    crate::user::key::send_device_key_update(&device.user_id, &device_id).await?;
    if created {
        res.status_code(StatusCode::CREATED);
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
                && matches!(
                    e.kind,
                    ErrorKind::Forbidden | ErrorKind::LimitExceeded { .. }
                )
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
    use crate::core::{UnixMillis, UserId};

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn http_appservice_device_put_preserves_requested_identity() {
        use salvo::test::{ResponseExt, TestClient};
        use serde_json::json;
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"device.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let registration = serde_json::from_value(json!({
            "id":"device-identity-test", "url":null,
            "as_token":"device-identity-fixture-token", "hs_token":"fixture-hs-token",
            "sender_localpart":"crypto_sender", "io.element.msc4190":true,
            "namespaces":{"users":[{"exclusive":true,"regex":"^@crypto_.*:device\\.example$"}],"aliases":[],"rooms":[]}
        })).unwrap();
        crate::appservice::register_appservice(registration)
            .await
            .unwrap();
        let service = Service::new(crate::routing::root());
        let user = UserId::parse("@crypto_child:device.example").unwrap();
        let requested = "HAGENCY_EXACT_DEVICE";
        let url = format!("http://localhost/_matrix/client/v3/devices/{requested}?user_id={user}");
        let mut response = TestClient::put(&url)
            .add_header(
                "Authorization",
                "Bearer device-identity-fixture-token",
                true,
            )
            .json(&json!({"display_name":"original name"}))
            .send(&service)
            .await;
        let status = response.status_code;
        let body = response.take_json::<serde_json::Value>().await.unwrap();
        assert_eq!(status, Some(StatusCode::CREATED), "{body}");
        let devices = data::user::device::get_devices(&user).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id.as_str(), requested);
        assert_eq!(devices[0].display_name.as_deref(), Some("original name"));
        assert!(
            data::user::authenticate_token(
                "device-identity-fixture-token",
                std::time::Duration::ZERO
            )
            .await
            .unwrap()
            .is_none()
        );

        let mut response = TestClient::get(format!(
            "http://localhost/_matrix/client/v3/account/whoami?user_id={user}&device_id={requested}"
        ))
        .add_header(
            "Authorization",
            "Bearer device-identity-fixture-token",
            true,
        )
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let who = response.take_json::<serde_json::Value>().await.unwrap();
        assert_eq!(who["user_id"], user.as_str());
        assert_eq!(who["device_id"], requested);

        let response = TestClient::put(&url)
            .add_header(
                "Authorization",
                "Bearer device-identity-fixture-token",
                true,
            )
            .json(&json!({"display_name":"updated name"}))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let devices = data::user::device::get_devices(&user).await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device_id.as_str(), requested);
        assert_eq!(devices[0].display_name.as_deref(), Some("updated name"));

        let response = TestClient::put(&url)
            .add_header(
                "Authorization",
                "Bearer device-identity-fixture-token",
                true,
            )
            .json(&json!({}))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));

        let second = UserId::parse("@crypto_second:device.example").unwrap();
        let response = TestClient::put(format!(
            "http://localhost/_matrix/client/v3/devices/{requested}?user_id={second}"
        ))
        .add_header(
            "Authorization",
            "Bearer device-identity-fixture-token",
            true,
        )
        .json(&json!({"display_name":"second owner"}))
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::CREATED));
        let second_devices = data::user::device::get_devices(&second).await.unwrap();
        assert_eq!(second_devices.len(), 1);
        assert_eq!(second_devices[0].device_id.as_str(), requested);
        assert_eq!(
            second_devices[0].display_name.as_deref(),
            Some("second owner")
        );
        assert_eq!(
            data::user::device::get_devices(&user).await.unwrap()[0]
                .display_name
                .as_deref(),
            Some("updated name")
        );

        let foreign = UserId::parse("@human:device.example").unwrap();
        let response = TestClient::put(format!(
            "http://localhost/_matrix/client/v3/devices/{requested}?user_id={foreign}"
        ))
        .add_header(
            "Authorization",
            "Bearer device-identity-fixture-token",
            true,
        )
        .json(&json!({"display_name":"forbidden"}))
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
        assert!(!data::user::user_exists(&foreign).await.unwrap());

        let registration = serde_json::from_value(json!({
            "id":"device-no-capability-test", "url":null,
            "as_token":"device-no-capability-fixture-token", "hs_token":"fixture-hs-token-2",
            "sender_localpart":"plain_sender", "io.element.msc4190":false,
            "namespaces":{"users":[{"exclusive":true,"regex":"^@plain_.*:device\\.example$"}],"aliases":[],"rooms":[]}
        })).unwrap();
        crate::appservice::register_appservice(registration)
            .await
            .unwrap();
        let plain = UserId::parse("@plain_child:device.example").unwrap();
        let response = TestClient::put(format!(
            "http://localhost/_matrix/client/v3/devices/{requested}?user_id={plain}"
        ))
        .add_header(
            "Authorization",
            "Bearer device-no-capability-fixture-token",
            true,
        )
        .json(&json!({"display_name":"forbidden"}))
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::NOT_FOUND));
        assert!(
            data::user::device::get_devices(&plain)
                .await
                .unwrap()
                .is_empty()
        );

        let concurrent_url =
            format!("http://localhost/_matrix/client/v3/devices/HAGENCY_CONCURRENT?user_id={user}");
        let send = || {
            TestClient::put(&concurrent_url)
                .add_header(
                    "Authorization",
                    "Bearer device-identity-fixture-token",
                    true,
                )
                .json(&json!({"display_name":"concurrent"}))
                .send(&service)
        };
        let (one, two) = tokio::join!(send(), send());
        assert!(matches!(
            (one.status_code, two.status_code),
            (Some(StatusCode::CREATED), Some(StatusCode::OK))
                | (Some(StatusCode::OK), Some(StatusCode::CREATED))
        ));
        let devices = data::user::device::get_devices(&user).await.unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(
            devices
                .iter()
                .filter(|device| device.device_id == "HAGENCY_CONCURRENT")
                .count(),
            1
        );
        assert!(
            data::user::authenticate_token(
                "device-identity-fixture-token",
                std::time::Duration::ZERO
            )
            .await
            .unwrap()
            .is_none()
        );
        crate::appservice::set_appservice_disabled("device-identity-test", true)
            .await
            .unwrap();
        let response = TestClient::put(&url)
            .add_header(
                "Authorization",
                "Bearer device-identity-fixture-token",
                true,
            )
            .json(&json!({"display_name":"disabled"}))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::UNAUTHORIZED));
        assert_eq!(
            data::user::device::get_devices(&user)
                .await
                .unwrap()
                .iter()
                .find(|device| device.device_id == requested)
                .unwrap()
                .display_name
                .as_deref(),
            Some("updated name")
        );
    }

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
        let changed =
            update_device_metadata_if_present(&alice, &shared, Some("alice-new"), "alice-ip")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(changed.user_id, alice);
        assert!(
            update_device_metadata_if_present(&alice, &bob_only, Some("unauthorized"), "wrong-ip")
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
