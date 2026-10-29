use palpo_core::client::account::ChangePasswordReqBody;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo};
use crate::exts::*;
use crate::{AuthArgs, EmptyResult, MatrixError, empty_ok};

pub fn authed_router() -> Router {
    Router::with_path("password").post(change_password)
}

/// #POST /_matrix/client/r0/account/password
/// Changes the password of this account.
///
/// - Requires UIAA to verify user password
/// - Changes the password of the sender user
/// - The password hash is calculated using argon2 with 32 character salt, the plain password is
/// not saved
///
/// If logout_devices is true it does the following for each device except the sender device:
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
#[endpoint]
async fn change_password(
    _req: &mut Request,
    _aa: AuthArgs,
    body: JsonBody<ChangePasswordReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    if crate::config::get().enabled_delegated_auth().is_some() {
        return Err(MatrixError::forbidden(
            "Password changes are handled by the delegated authentication service.",
            None,
        )
        .into());
    }

    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: Default::default(),
        session: None,
        auth_error: None,
    };
    let Some(auth) = &body.auth else {
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
        auth,
        &uiaa_info,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            if matches!(
                &error,
                crate::AppError::Matrix(MatrixError {
                    kind: crate::core::error::ErrorKind::LimitExceeded { .. },
                    ..
                })
            ) {
                return Err(error);
            }
            crate::uiaa::create_challenge_session(
                authed.user_id(),
                authed.require_device_id()?,
                &mut uiaa_info,
            )
            .await?;
            return Err(uiaa_info.into());
        }
    };
    if !authenticated {
        return Err(uiaa.into());
    }

    crate::user::set_password(authed.user_id(), &body.new_password).await?;
    if body.logout_devices {
        // Device removal also revokes encryption keys and queued messages, and
        // publishes the device-list change to local clients and federation.
        for device_id in crate::data::user::all_device_ids(authed.user_id()).await? {
            if device_id != authed.require_device_id()? {
                crate::user::remove_device(authed.user_id(), &device_id).await?;
            }
        }
    }

    info!("User {} changed their password.", authed.user_id());
    // crate::admin::send_message(RoomMessageEventContent::notice_plain(format!("User {user} changed
    // their password.")));

    empty_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::encryption::{DeviceKeys, OneTimeKey};
    use crate::core::{DeviceKeyAlgorithm, DeviceKeyId, OwnedDeviceId, UnixMillis, UserId};
    use crate::data::{connect, schema::*};
    use diesel::prelude::*;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_password_logout_removes_device_keys_and_rolls_back_failed_cleanup() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"password.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let user =
            UserId::parse(format!("@passwordrevoke:{}", crate::config::server_name())).unwrap();
        crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user.localpart().into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        crate::user::set_password(&user, "old password")
            .await
            .unwrap();
        let current: OwnedDeviceId = "REVOKE_CURRENT".into();
        let other: OwnedDeviceId = "REVOKE_OTHER".into();
        for (device, token) in [(&current, "revoke-current"), (&other, "revoke-other")] {
            crate::data::user::device::create_device(&user, device, token, None, None)
                .await
                .unwrap();
            let keys: DeviceKeys = serde_json::from_value(json!({
                "user_id":user,"device_id":device,"algorithms":[],"keys":{},"signatures":{}
            }))
            .unwrap();
            crate::data::user::key::add_device_keys(&user, device, &keys)
                .await
                .unwrap();
            let key_id = DeviceKeyId::parse("signed_curve25519:revoke").unwrap();
            let key: OneTimeKey = serde_json::from_value(json!("cmV2b2tl")).unwrap();
            crate::data::user::key::add_one_time_key(&user, device, &key_id, &key)
                .await
                .unwrap();
            crate::data::user::key::add_fallback_key(&user, device, &key_id, &key)
                .await
                .unwrap();
            crate::data::user::device::add_to_device_event(
                &user,
                &user,
                device,
                "m.test",
                json!({}),
            )
            .await
            .unwrap();
        }
        crate::data::user::device::set_refresh_token(
            &user,
            &other,
            "revoke-refresh",
            u64::MAX / 2,
            u64::MAX / 2,
        )
        .await
        .unwrap();
        crate::data::user::pusher::set_pusher(
            &crate::data::user::pusher::NewDbPusher {
                user_id: user.clone(),
                kind: "http".into(),
                app_id: "revoke".into(),
                app_display_name: "fixture".into(),
                device_id: other.clone(),
                device_display_name: "fixture".into(),
                access_token_id: None,
                profile_tag: None,
                pushkey: "revoke-push".into(),
                lang: "en".into(),
                data: json!({}),
                enabled: true,
                created_at: UnixMillis::now(),
            },
            false,
        )
        .await
        .unwrap();

        // A late cleanup failure must roll back the earlier metadata/token/key deletes.
        connect().await.unwrap().batch_execute("CREATE FUNCTION q07_fail_cleanup() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.device_id = 'REVOKE_OTHER' THEN RAISE EXCEPTION 'injected cleanup failure'; END IF; RETURN OLD; END $$; CREATE TRIGGER q07_fail_cleanup BEFORE DELETE ON device_inboxes FOR EACH ROW EXECUTE FUNCTION q07_fail_cleanup();").await.unwrap();
        assert!(
            crate::data::user::device::remove_device(&user, &other)
                .await
                .is_err()
        );
        assert!(
            crate::data::user::device::is_device_exists(&user, &other)
                .await
                .unwrap()
        );
        assert!(
            crate::data::user::get_device_keys(&user, &other)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            user_access_tokens::table
                .filter(user_access_tokens::user_id.eq(&user))
                .count()
                .get_result::<i64>(&mut connect().await.unwrap())
                .await
                .unwrap(),
            2
        );
        connect().await.unwrap().batch_execute("DROP TRIGGER q07_fail_cleanup ON device_inboxes; DROP FUNCTION q07_fail_cleanup();").await.unwrap();

        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(authed_router()),
        );
        let mut response = TestClient::post("http://localhost/password")
            .add_header("authorization", "Bearer revoke-current", true)
            .json(&json!({"new_password":"new password","logout_devices":true,
                "auth":{"type":"m.login.password","identifier":{"type":"m.id.user","user":user},"password":"old password"}}))
            .send(&service).await;
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
        assert!(
            crate::data::user::device::is_device_exists(&user, &current)
                .await
                .unwrap()
        );
        assert!(
            !crate::data::user::device::is_device_exists(&user, &other)
                .await
                .unwrap()
        );
        assert!(
            crate::data::user::get_device_keys(&user, &other)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            crate::data::user::get_device_keys(&user, &current)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            crate::data::user::key::claim_one_time_key(
                &user,
                &other,
                &DeviceKeyAlgorithm::SignedCurve25519
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(
            crate::data::user::device::get_to_device_events(&user, &other, None, None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            crate::data::user::pusher::get_pushers(&user)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            user_refresh_tokens::table
                .filter(user_refresh_tokens::user_id.eq(&user))
                .count()
                .get_result::<i64>(&mut connect().await.unwrap())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            user_access_tokens::table
                .filter(user_access_tokens::user_id.eq(&user))
                .count()
                .get_result::<i64>(&mut connect().await.unwrap())
                .await
                .unwrap(),
            1
        );
        assert!(
            crate::data::user::key::device_list_stream_id(&user)
                .await
                .unwrap()
                > 0
        );
        assert!(
            !crate::data::user::keys_changed_users(&user, 0, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
