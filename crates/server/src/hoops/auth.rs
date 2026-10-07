use std::collections::BTreeMap;
use std::iter::FromIterator;
use std::time::Duration;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use salvo::http::headers::HeaderMapExt;
use salvo::http::headers::authorization::Authorization;
use salvo::prelude::*;

use crate::core::federation::authentication::XMatrix;
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonValue;
use crate::core::{UnixMillis, signatures};
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::{DbUser, DbUserDevice, NewDbProfile, NewDbUser};
use crate::exts::DepotExt;
use crate::server_key::{PubKeyMap, PubKeys};
use crate::{AppError, AppResult, AuthArgs, AuthedInfo, MatrixError, config};

#[handler]
pub async fn auth_by_access_token_or_signatures(
    aa: AuthArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> AppResult<()> {
    if aa.uses_access_token() {
        auth_by_access_token_inner(aa, depot).await
    } else if aa.authorization.is_some() {
        auth_by_signatures_inner(req, depot).await
    } else {
        Err(MatrixError::missing_token("missing token").into())
    }
}

#[handler]
pub async fn auth_by_access_token(aa: AuthArgs, depot: &mut Depot) -> AppResult<()> {
    auth_by_access_token_inner(aa, depot).await
}

/// Authenticates a route whose own query schema uses `user_id` for something other than
/// application-service masquerading.
///
/// Matrix's stable mutual-rooms endpoint names its target-user query parameter `user_id`,
/// which otherwise collides with the application-service impersonation parameter parsed
/// into [`AuthArgs`]. Such a route must authenticate the application service as its sender
/// and leave the target `user_id` for the endpoint extractor.
#[handler]
pub async fn auth_by_access_token_without_query_masquerade(
    mut aa: AuthArgs,
    depot: &mut Depot,
) -> AppResult<()> {
    aa.user_id = None;
    auth_by_access_token_inner(aa, depot).await
}

#[handler]
pub async fn auth_by_signatures(
    _aa: AuthArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> AppResult<()> {
    auth_by_signatures_inner(req, depot).await
}

async fn auth_by_access_token_inner(aa: AuthArgs, depot: &mut Depot) -> AppResult<()> {
    let token = aa.require_access_token()?;

    if auth_by_local_token(token, &aa, depot).await? {
        return Ok(());
    }

    // Delegated auth coexists with local tokens. Only fall back to external
    // introspection after the token misses Palpo's local access/appservice
    // stores, preserving existing password sessions when OIDC is enabled.
    if config::get().enabled_delegated_auth().is_some() {
        return auth_by_delegated_token(token, &aa, depot).await;
    }

    Err(MatrixError::unknown_token("unknown access token", true).into())
}

async fn auth_by_local_token(token: &str, aa: &AuthArgs, depot: &mut Depot) -> AppResult<bool> {
    // Native auth: resolve the token to its user/device, optionally via an
    // in-memory cache (disabled when native_token_cache_ttl is 0). `None` means
    // it isn't a user access token, so we fall through to the appservice token
    // scheme below.
    let cache_ttl = Duration::from_secs(config::get().native_token_cache_ttl);
    let token_auth = crate::data::user::authenticate_token(token, cache_ttl)
        .await
        .map_err(|e| {
            tracing::error!("failed to query access token: {e}");
            MatrixError::unknown("Internal server error during authentication")
        })?;
    if let Some(crate::data::user::TokenAuth {
        user,
        device,
        access_token_id,
    }) = token_auth
    {
        crate::user::ensure_account_usable(&user)?;
        depot.insert_typed(AuthedInfo {
            user,
            user_device: Some(device),
            access_token_id: Some(access_token_id),
            appservice: None,
        });
        Ok(true)
    } else {
        // Import file-backed registrations once, then authenticate against the
        // enabled database registrations. The startup list excludes registrations
        // added through the admin API and cannot reflect disable/delete changes.
        crate::appservices().await;
        if let Some(appservice_info) = crate::appservice::find_from_token(token).await? {
            let appservice = &appservice_info.registration;
            let user_id = if let Some(ref user_id_str) = aa.user_id {
                let user_id = UserId::parse(user_id_str)
                    .map_err(|_| MatrixError::invalid_param("Invalid user_id"))?;
                if !appservice_info.is_user_match(&user_id) {
                    return Err(MatrixError::forbidden(
                        "User is not in appservice's namespace",
                        None,
                    )
                    .into());
                }
                user_id
            } else {
                // A dynamically registered service may not have a sender account
                // yet. Resolve the exact sender, never an arbitrary virtual user.
                UserId::parse_with_server_name(
                    appservice.sender_localpart.as_str(),
                    &config::get().server_name,
                )
                .map_err(|_| MatrixError::invalid_param("Invalid appservice sender_localpart"))?
            };
            let user_device = resolve_appservice_device(&user_id, aa.device_id.as_deref()).await?;
            let user = get_or_create_appservice_user(&user_id, &appservice.id).await?;

            crate::user::ensure_account_usable(&user)?;
            depot.insert_typed(AuthedInfo {
                user,
                user_device,
                access_token_id: None,
                appservice: Some(appservice_info),
            });
            return Ok(true);
        }
        Ok(false)
    }
}

/// Validate a token via the external authorization server's introspection endpoint.
async fn auth_by_delegated_token(token: &str, _aa: &AuthArgs, depot: &mut Depot) -> AppResult<()> {
    let result = super::introspection::introspect_token(token).await?;

    if !result.active {
        return Err(MatrixError::unknown_token("Token is not active", true).into());
    }

    let scope = result
        .scope
        .as_deref()
        .ok_or_else(|| MatrixError::unknown_token("Token has no Matrix API scope", true))?;
    if !super::introspection::has_matrix_api_scope(scope) {
        return Err(MatrixError::unknown_token("Token has no Matrix API scope", true).into());
    }
    let device_id_str = super::introspection::device_id_from_scope(scope)
        .ok_or_else(|| MatrixError::unknown_token("Token has no unique Matrix device", true))?;
    if result
        .device_id
        .as_deref()
        .is_some_and(|device_id| device_id != device_id_str.as_str())
    {
        return Err(MatrixError::unknown_token("Token has mismatched Matrix device", true).into());
    }

    let username = result
        .username
        .as_deref()
        .ok_or_else(|| MatrixError::unknown_token("No username in introspection response", true))?;

    let conf = config::get();
    let user_id = UserId::parse_with_server_name(username, &conf.server_name)
        .map_err(|_| MatrixError::unknown_token("Invalid username in token", true))?;

    // User should already be provisioned by the auth service via MAS admin endpoints
    let mut user = users::table
        .find(&user_id)
        .first::<DbUser>(&mut connect().await?)
        .await
        .map_err(|_| MatrixError::unknown_token("User not found (not yet provisioned?)", true))?;
    if user.is_guest {
        crate::data::user::set_guest(&user_id, false).await?;
        user.is_guest = false;
    }
    crate::user::ensure_account_usable(&user)?;

    let device_id: OwnedDeviceId = device_id_str.into();
    let user_device = user_devices::table
        .filter(user_devices::user_id.eq(&user_id))
        .filter(user_devices::device_id.eq(&device_id))
        .first::<DbUserDevice>(&mut connect().await?)
        .await
        .map_err(|_| MatrixError::unknown_token("Device not found (not yet provisioned?)", true))?;

    depot.insert_typed(AuthedInfo {
        user,
        user_device: Some(user_device),
        access_token_id: None,
        appservice: None,
    });
    Ok(())
}

/// Get or create a user for an appservice
async fn get_or_create_appservice_user(user_id: &UserId, appservice_id: &str) -> AppResult<DbUser> {
    // Try to get existing user
    if let Some(user) = users::table
        .find(user_id)
        .first::<DbUser>(&mut connect().await?)
        .await
        .optional()?
    {
        return Ok(user);
    }

    // Create new user for the appservice
    let new_user = NewDbUser {
        id: user_id.to_owned(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: user_id.server_name() == config::get().server_name,
        localpart: user_id.localpart().to_owned(),
        server_name: user_id.server_name().to_owned(),
        appservice_id: Some(appservice_id.to_owned()),
        created_at: UnixMillis::now(),
    };

    connect()
        .await?
        .transaction::<_, AppError, _>(async |conn| {
            // The upsert locks the user row until this transaction commits. Concurrent
            // first requests for the same virtual user therefore serialize before the
            // profile existence check, despite NULL room IDs not being unique in Postgres.
            let user = diesel::insert_into(users::table)
                .values(&new_user)
                .on_conflict(users::id)
                .do_update()
                .set(&new_user)
                .get_result::<DbUser>(conn)
                .await?;

            let profile_exists = user_profiles::table
                .filter(user_profiles::user_id.eq(user_id))
                .filter(user_profiles::room_id.is_null())
                .select(user_profiles::id)
                .first::<i64>(conn)
                .await
                .optional()?
                .is_some();
            if !profile_exists {
                diesel::insert_into(user_profiles::table)
                    .values(&NewDbProfile {
                        user_id: user_id.to_owned(),
                        room_id: None,
                        display_name: Some(user_id.localpart().to_owned()),
                        avatar_url: None,
                        blurhash: None,
                    })
                    .execute(conn)
                    .await?;
            }

            Ok(user)
        })
        .await
}

/// Resolve an asserted device without provisioning or creating any device.
async fn resolve_appservice_device(
    user_id: &UserId,
    device_id: Option<&str>,
) -> AppResult<Option<DbUserDevice>> {
    let Some(device_id) = device_id else {
        return Ok(None);
    };
    let device_id: OwnedDeviceId = device_id.into();
    let device = user_devices::table
        .filter(user_devices::user_id.eq(user_id))
        .filter(user_devices::device_id.eq(&device_id))
        .first::<DbUserDevice>(&mut connect().await?)
        .await
        .optional()?;
    device.map(Some).ok_or_else(|| {
        MatrixError::unknown_device("The asserted device does not belong to this user.").into()
    })
}
async fn auth_by_signatures_inner(req: &mut Request, depot: &mut Depot) -> AppResult<()> {
    let Some(Authorization(x_matrix)) = req.headers().typed_get::<Authorization<XMatrix>>() else {
        warn!("missing or invalid Authorization header");
        return Err(MatrixError::forbidden("Missing or invalid authorization header", None).into());
    };

    let origin_signatures = BTreeMap::from_iter([(
        x_matrix.key.as_str().to_owned(),
        CanonicalJsonValue::String(x_matrix.sig.to_string()),
    )]);

    let origin = &x_matrix.origin;
    if !config::get().federation.is_server_allowed(origin) {
        return Err(
            MatrixError::forbidden("Federation with this server is not allowed.", None).into(),
        );
    }

    let signatures = BTreeMap::from_iter([(
        origin.as_str().to_owned(),
        CanonicalJsonValue::Object(origin_signatures),
    )]);

    let mut authorization = BTreeMap::from_iter([
        (
            "destination".to_owned(),
            CanonicalJsonValue::String(config::get().server_name.as_str().to_owned()),
        ),
        (
            "method".to_owned(),
            CanonicalJsonValue::String(req.method().to_string()),
        ),
        (
            "origin".to_owned(),
            CanonicalJsonValue::String(origin.as_str().to_owned()),
        ),
        (
            "uri".to_owned(),
            format!(
                "{}{}",
                req.uri().path(),
                req.uri()
                    .query()
                    .map(|q| format!("?{q}"))
                    .unwrap_or_default()
            )
            .into(),
        ),
        (
            "signatures".to_owned(),
            CanonicalJsonValue::Object(signatures),
        ),
    ]);

    let json_body = match req.payload().await {
        Ok(payload) => match serde_json::from_slice::<CanonicalJsonValue>(payload) {
            Ok(json) => Some(json),
            Err(e) => {
                debug!("failed to parse federation request body as JSON: {e}");
                None
            }
        },
        Err(e) => {
            debug!("failed to read federation request payload: {e}");
            None
        }
    };

    if let Some(json_body) = &json_body {
        authorization.insert("content".to_owned(), json_body.clone());
    };

    let key = crate::server_key::get_verify_key(origin, &x_matrix.key).await?;

    let keys: PubKeys = [(x_matrix.key.to_string(), key.key)].into();
    let keys: PubKeyMap = [(origin.as_str().into(), keys)].into();
    if let Err(e) = signatures::verify_json(&keys, &authorization) {
        warn!(
            "Failed to verify json request from {}: {}\n{:?}",
            x_matrix.origin, e, authorization
        );

        if req.uri().to_string().contains('@') {
            warn!(
                "Request uri contained '@' character. Make sure your \
                                         reverse proxy gives Palpo the raw uri (apache: use \
                                         nocanon)"
            );
        }

        Err(MatrixError::forbidden("Failed to verify X-Matrix signatures.", None).into())
    } else {
        depot.set_origin(origin.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::appservice::Registration;
    use crate::core::error::ErrorKind;

    async fn authenticate_fixture(token: &str, user_id: Option<&str>) -> AppResult<AuthedInfo> {
        authenticate_fixture_device(token, user_id, None).await
    }

    async fn authenticate_fixture_device(
        token: &str,
        user_id: Option<&str>,
        device_id: Option<&str>,
    ) -> AppResult<AuthedInfo> {
        let args = AuthArgs {
            user_id: user_id.map(ToOwned::to_owned),
            device_id: device_id.map(ToOwned::to_owned),
            access_token: None,
            authorization: Some(format!("Bearer {token}")),
            from_appservice: false,
        };
        let mut depot = Depot::new();
        auth_by_access_token_inner(args, &mut depot).await?;
        depot.take_authed_info()
    }

    fn assert_unknown_token(result: AppResult<AuthedInfo>) {
        assert!(matches!(
            result,
            Err(AppError::Matrix(MatrixError {
                kind: ErrorKind::UnknownToken { .. },
                ..
            }))
        ));
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_dynamic_appservice_auth() {
        crate::test_database::init();
        config::CONFIG.get_or_init(|| {
            serde_json::from_value(serde_json::json!({
                "server_name": "dynamic.example", "db": { "url": "unused-test-config" }
            }))
            .unwrap()
        });
        // Reproduce a running server whose file registration list was initialized
        // before an administrator installs an application service.
        assert!(crate::appservices().await.is_empty());
        let token = "unprefixed_base64url-fixture-token_123";
        assert_unknown_token(authenticate_fixture(token, None).await);
        let registration: Registration = serde_json::from_value(serde_json::json!({
            "id": "dynamic-auth-fixture", "url": null,
            "as_token": token, "hs_token": "fixture-homeserver-token",
            "sender_localpart": "dynamic_sender",
            "namespaces": { "users": [{ "exclusive": true, "regex": "^@dynamic_.*:dynamic\\.example$" }], "aliases": [], "rooms": [] }
        })).unwrap();
        crate::appservice::register_appservice(registration.clone())
            .await
            .unwrap();
        let stored = crate::appservice::get_registration(&registration.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.as_token, token);

        // Create a virtual user first: a request without user_id must still use
        // sender_localpart, not whichever appservice user the database finds first.
        let child = authenticate_fixture(token, Some("@dynamic_child:dynamic.example"))
            .await
            .unwrap();
        assert_eq!(child.user_id().as_str(), "@dynamic_child:dynamic.example");
        assert_eq!(child.appservice().unwrap().registration.id, registration.id);
        let sender = authenticate_fixture(token, None).await.unwrap();
        assert_eq!(sender.user_id().as_str(), "@dynamic_sender:dynamic.example");
        assert_eq!(
            sender.user.appservice_id.as_deref(),
            Some(registration.id.as_str())
        );
        assert!(sender.access_token_id().is_none());
        let repeated = authenticate_fixture(token, None).await.unwrap();
        assert!(repeated.user_device.is_none());
        assert!(sender.user_device.is_none());

        assert!(
            crate::data::user::device::get_devices(sender.user_id())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            crate::data::user::device::get_devices(child.user_id())
                .await
                .unwrap()
                .is_empty()
        );
        for (user_id, device_id) in [(sender.user_id(), "SENDER"), (child.user_id(), "CHILD")] {
            diesel::insert_into(user_devices::table)
                .values(crate::data::user::NewDbUserDevice {
                    user_id: user_id.to_owned(),
                    device_id: device_id.into(),
                    display_name: None,
                    user_agent: None,
                    is_hidden: false,
                    last_seen_ip: None,
                    last_seen_at: None,
                    created_at: UnixMillis::now(),
                })
                .execute(&mut connect().await.unwrap())
                .await
                .unwrap();
        }
        let asserted =
            authenticate_fixture_device(token, Some(child.user_id().as_str()), Some("CHILD"))
                .await
                .unwrap();
        assert_eq!(asserted.device_id().unwrap().as_str(), "CHILD");
        let asserted_sender = authenticate_fixture_device(token, None, Some("SENDER"))
            .await
            .unwrap();
        assert_eq!(asserted_sender.device_id().unwrap().as_str(), "SENDER");
        for (user_id, device_id) in [
            (None, "CHILD"),
            (Some(child.user_id().as_str()), "SENDER"),
            (Some("@dynamic_new:dynamic.example"), "UNKNOWN"),
        ] {
            assert!(matches!(
                authenticate_fixture_device(token, user_id, Some(device_id)).await,
                Err(AppError::Matrix(MatrixError {
                    kind: ErrorKind::UnknownDevice,
                    ..
                }))
            ));
        }
        assert!(
            !crate::data::user::user_exists(
                &UserId::parse("@dynamic_new:dynamic.example").unwrap()
            )
            .await
            .unwrap()
        );

        use salvo::test::{ResponseExt, TestClient};
        let service = Service::new(crate::routing::root());
        let whoami = "http://localhost/_matrix/client/v3/account/whoami";
        let mut response = TestClient::get(whoami)
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let json = response.take_json::<serde_json::Value>().await.unwrap();
        assert_eq!(json["user_id"], sender.user_id().as_str());
        assert!(json.get("device_id").is_none());
        let mut unknown = TestClient::get(format!("{whoami}?device_id=CHILD"))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(&service)
            .await;
        assert_eq!(unknown.status_code, Some(StatusCode::BAD_REQUEST));
        assert_eq!(
            unknown.take_json::<serde_json::Value>().await.unwrap()["errcode"],
            "M_UNKNOWN_DEVICE"
        );
        let mut keys = TestClient::post("http://localhost/_matrix/client/v3/keys/upload")
            .json(&serde_json::json!({}))
            .add_header("Authorization", format!("Bearer {token}"), true)
            .send(&service)
            .await;
        assert_eq!(keys.status_code, Some(StatusCode::BAD_REQUEST));
        assert_eq!(
            keys.take_json::<serde_json::Value>().await.unwrap()["errcode"],
            "M_MISSING_PARAM"
        );
        assert_eq!(
            crate::data::user::device::get_devices(sender.user_id())
                .await
                .unwrap()
                .len(),
            1
        );
        // Ordinary tokens retain their real device even when assertion query parameters exist.
        crate::data::user::device::create_device(
            sender.user_id(),
            &OwnedDeviceId::from("NATIVE"),
            "native-device-fixture-token",
            None,
            None,
        )
        .await
        .unwrap();
        let mut native = TestClient::get(format!("{whoami}?device_id=UNKNOWN"))
            .add_header("Authorization", "Bearer native-device-fixture-token", true)
            .send(&service)
            .await;
        assert_eq!(native.status_code, Some(StatusCode::OK));
        assert_eq!(
            native.take_json::<serde_json::Value>().await.unwrap()["device_id"],
            "NATIVE"
        );

        assert_unknown_token(authenticate_fixture("wrong-fixture-token", None).await);
        assert!(matches!(
            authenticate_fixture(token, Some("@outside:dynamic.example")).await,
            Err(AppError::Matrix(MatrixError {
                kind: ErrorKind::Forbidden,
                ..
            }))
        ));
        assert!(
            crate::appservice::set_appservice_disabled(&registration.id, true)
                .await
                .unwrap()
        );
        assert_unknown_token(authenticate_fixture(token, None).await);
        assert_unknown_token(
            authenticate_fixture(token, Some("@dynamic_child:dynamic.example")).await,
        );
        assert!(
            crate::appservice::set_appservice_disabled(&registration.id, false)
                .await
                .unwrap()
        );
        assert_eq!(
            authenticate_fixture(token, None).await.unwrap().user_id(),
            sender.user_id()
        );
        crate::appservice::unregister_appservice(&registration.id)
            .await
            .unwrap();
        assert_unknown_token(authenticate_fixture(token, None).await);
    }
}
