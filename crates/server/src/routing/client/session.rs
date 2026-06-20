use std::time::Duration;

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use palpo_data::user::set_display_name;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::UnixMillis;
use crate::core::client::session::*;
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo, UserIdentifier};
use crate::core::error::ErrorKind;
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonValue;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::{DbUser, DbUserDevice, NewDbUser};
use crate::exts::*;
use crate::{
    AppError, AppResult, AuthArgs, DEVICE_ID_LENGTH, DepotExt, EmptyResult, JsonResult,
    MatrixError, SESSION_ID_LENGTH, TOKEN_LENGTH, config, data, empty_ok, hoops, json_ok, user,
    utils,
};

pub fn public_router() -> Router {
    Router::new().push(
        Router::with_path("login")
            .hoop(hoops::limit_rate_login)
            .get(login_types)
            .post(login)
            .push(
                Router::with_path("sso/redirect")
                    .get(redirect)
                    .push(Router::with_path("idpId").get(provider_url)),
            ),
    )
}
pub fn authed_router() -> Router {
    Router::new()
        .push(
            Router::with_path("login")
                .hoop(hoops::limit_rate_login)
                .push(Router::with_path("get_token").post(get_access_token)),
        )
        .push(Router::with_path("refresh").post(refresh_access_token))
        .push(
            Router::with_path("logout")
                .post(logout)
                .push(Router::with_path("all").post(logout_all)),
        )
}

/// #GET /_matrix/client/r0/login
/// Get the supported login types of this server. One of these should be used as the `type` field
/// when logging in.
#[endpoint]
async fn login_types(_aa: AuthArgs) -> JsonResult<LoginTypesResBody> {
    let delegated_auth = config::get().enabled_delegated_auth();
    Ok(Json(LoginTypesResBody::new(supported_login_flows(
        delegated_auth.is_some(),
        delegated_auth
            .map(config::DelegatedAuthConfig::password_login_enabled)
            .unwrap_or(false),
    ))))
}

fn supported_login_flows(
    delegated_auth_enabled: bool,
    delegated_password_login_enabled: bool,
) -> Vec<LoginType> {
    let mut flows = Vec::new();
    if !delegated_auth_enabled || delegated_password_login_enabled {
        flows.push(LoginType::password());
    }
    flows.push(LoginType::appservice());
    if delegated_auth_enabled {
        flows.push(LoginType::Sso(
            crate::core::client::session::SsoLoginType::new(),
        ));
    }
    flows
}

/// #POST /_matrix/client/r0/login
/// Authenticates the user and returns an access token it can use in subsequent requests.
///
/// - The user needs to authenticate using their password (or if enabled using a json web token)
/// - If `device_id` is known: invalidates old access token of that device
/// - If `device_id` is unknown: creates a new device
/// - Returns access token that is associated with the user and device
///
/// Note: You can use [`GET /_matrix/client/r0/login`](fn.get_supported_versions_route.html) to see
/// supported login types.
#[endpoint]
async fn login(
    aa: AuthArgs,
    body: JsonBody<LoginReqBody>,
    req: &mut Request,
    res: &mut Response,
) -> JsonResult<LoginResBody> {
    // Validate login method
    // TODO: Other login methods
    let user_id = match &body.login_info {
        LoginInfo::Password(Password {
            identifier,
            password,
        }) => {
            let username = if let UserIdentifier::Matrix(user_id) = identifier {
                user_id.user.to_lowercase()
            } else {
                warn!("Bad login type: {:?}", &body.login_info);
                return Err(MatrixError::forbidden("Bad login type.", None).into());
            };
            let user_id = UserId::parse_with_server_name(username, &config::get().server_name)
                .map_err(|_| MatrixError::invalid_username("Username is invalid."))?;

            if let Some(da) = config::get().enabled_delegated_auth() {
                let device_id = body
                    .device_id
                    .clone()
                    .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());
                return delegated_password_login(
                    da,
                    &user_id,
                    password,
                    device_id,
                    body.initial_device_display_name.clone(),
                )
                .await;
            }

            // if let Some(ldap) = config::enabled_ldap() {
            //     let (user_dn, is_ldap_admin) = match ldap.bind_dn.as_ref() {
            //         Some(bind_dn) if bind_dn.contains("{username}") => {
            //             (bind_dn.replace("{username}", user_id.localpart()), false)
            //         }
            //         _ => {
            //             debug!("searching user in LDAP");

            //             let dns = user::search_ldap(&user_id).await?;
            //             if dns.len() >= 2 {
            //                 return Err(MatrixError::forbidden("LDAP search returned two or more
            // results", None).into());             }

            //             if let Some((user_dn, is_admin)) = dns.first() {
            //                 (user_dn.clone(), *is_admin)
            //             } else {
            //                 let Ok(user) = data::user::get_user(&user_id)? else {
            //                     return Err(MatrixError::forbidden("user not found.",
            // None).into());                 };
            //                 if let Err(_e) = user::vertify_password(&user, password) {
            //                     res.status_code(StatusCode::FORBIDDEN); //for complement testing:
            // TestLogin/parallel/POST_/login_wrong_password_is_rejected
            // return Err(MatrixError::forbidden("wrong username or password.", None).into());
            //                 }
            //                 (user_id.to_string(), false)
            //             }
            //         }
            //     };

            //     let user_id = user::auth_ldap(&user_dn, password).await.map(|()|
            // user_id.to_owned())?;

            //     // LDAP users are automatically created on first login attempt. This is a very
            //     // common feature that can be seen on many services using a LDAP provider for
            //     // their users (synapse, Nextcloud, Jellyfin, ...).
            //     //
            //     // LDAP users are crated with a dummy password but non empty because an empty
            //     // password is reserved for deactivated accounts. The palpo password field
            //     // will never be read to login a LDAP user so it's not an issue.
            //     if !data::user::user_exists(&user_id)? {
            //         let new_user = NewDbUser {
            //             id: user_id.clone(),
            //             ty: Some("ldap".to_owned()),
            //             is_admin: false,
            //             is_guest: false,
            //             appservice_id: None,
            //             created_at: UnixMillis::now(),
            //         };
            //         let user = diesel::insert_into(users::table)
            //             .values(&new_user)
            //             .on_conflict(users::id)
            //             .do_update()
            //             .set(&new_user)
            //             .get_result::<DbUser>(&mut connect()?)?;
            //     }

            //     let is_palpo_admin = data::user::is_admin(&user_id)?;
            //     if is_ldap_admin && !is_palpo_admin {
            //         admin::make_admin(&user_id).await?;
            //     } else if !is_ldap_admin && is_palpo_admin {
            //         admin::revoke_admin(&user_id).await?;
            //     }
            // } else {
            let Ok(user) = data::user::get_user(&user_id).await else {
                return Err(MatrixError::forbidden("User not found.", None).into());
            };
            if let Err(e) = user::verify_password(&user, password).await {
                res.status_code(StatusCode::FORBIDDEN); //for complement testing: TestLogin/parallel/POST_/login_wrong_password_is_rejected
                if let AppError::Matrix(matrix) = e {
                    if matches!(
                        matrix.kind,
                        ErrorKind::UserDeactivated
                            | ErrorKind::UserLocked
                            | ErrorKind::UserSuspended
                    ) {
                        return Err(matrix.into());
                    }
                }
                return Err(MatrixError::forbidden("Wrong username or password.", None).into());
            }
            // }

            user_id
        }
        LoginInfo::Token(Token { token }) => {
            if !crate::config::get().login_via_existing_session {
                return Err(MatrixError::unknown("Token login is not enabled.").into());
            }
            user::take_login_token(token).await?
        }
        LoginInfo::Jwt(info) => {
            let conf = config::get();
            let jwt_conf = conf
                .enabled_jwt()
                .ok_or_else(|| MatrixError::unknown("JWT login is not enabled."))?;

            let claim = user::session::validate_jwt_token(jwt_conf, &info.token)?;
            let local = claim.sub.to_lowercase();
            let user_id =
                UserId::parse_with_server_name(local, &conf.server_name).map_err(|e| {
                    MatrixError::invalid_username(format!(
                        "JWT subject is not a valid user MXID: {e}"
                    ))
                })?;

            if !data::user::user_exists(&user_id).await? {
                if !jwt_conf.register_user {
                    return Err(
                        MatrixError::not_found("user is not registered on this server.").into(),
                    );
                }

                let new_user = NewDbUser {
                    id: user_id.clone(),
                    ty: Some("jwt".to_owned()),
                    is_admin: false,
                    is_guest: false,
                    is_local: user_id.server_name().is_local(),
                    localpart: user_id.localpart().to_string(),
                    server_name: user_id.server_name().to_owned(),
                    appservice_id: None,
                    created_at: UnixMillis::now(),
                };
                let user = diesel::insert_into(users::table)
                    .values(&new_user)
                    .on_conflict(users::id)
                    .do_update()
                    .set(&new_user)
                    .get_result::<DbUser>(&mut connect().await?)
                    .await?;

                // Set initial user profile
                if let Err(e) = set_display_name(&user.id, user.id.localpart()).await {
                    tracing::warn!("failed to set profile for new user (non-fatal): {}", e);
                }
            }
            user_id
        }
        LoginInfo::Appservice(Appservice { identifier }) => {
            authenticate_appservice_login(identifier, &aa).await?
        }
        _ => {
            warn!("Unsupported or unknown login type: {:?}", &body.login_info);
            return Err(MatrixError::unknown("Unsupported login type.").into());
        }
    };

    let user = data::user::get_user(&user_id)
        .await
        .map_err(|_| MatrixError::forbidden("User not found.", None))?;
    user::ensure_account_usable(&user)?;

    // Generate new device id if the user didn't specify one
    let device_id = body
        .device_id
        .clone()
        .unwrap_or_else(|| utils::random_string(DEVICE_ID_LENGTH).into());

    // Generate a new token for the device
    let access_token = utils::random_string(TOKEN_LENGTH);

    let (refresh_token, refresh_token_id) = if body.refresh_token {
        let refresh_token = utils::random_string(TOKEN_LENGTH);
        let expires_at = UnixMillis::now().get() + crate::config::get().refresh_token_ttl;
        let ultimate_session_expires_at =
            UnixMillis::now().get() + crate::config::get().session_ttl;
        let refresh_token_id = data::user::device::set_refresh_token(
            &user_id,
            &device_id,
            &refresh_token,
            expires_at,
            ultimate_session_expires_at,
        )
        .await?;
        (Some(refresh_token), Some(refresh_token_id))
    } else {
        (None, None)
    };

    // Determine if device_id was provided and exists in the db for this user
    if data::user::device::is_device_exists(&user_id, &device_id).await? {
        data::user::device::set_access_token(&user_id, &device_id, &access_token, refresh_token_id)
            .await?;
    } else {
        data::user::device::create_device(
            &user_id,
            &device_id,
            &access_token,
            body.initial_device_display_name.clone(),
            Some(req.remote_addr().to_string()),
        )
        .await?;
    }

    tracing::info!("{} logged in", user_id);

    json_ok(LoginResBody {
        user_id,
        access_token,
        device_id,
        well_known: None,
        refresh_token,
        expires_in: None,
    })
}

async fn authenticate_appservice_login(
    identifier: &UserIdentifier,
    aa: &AuthArgs,
) -> AppResult<OwnedUserId> {
    let user_id = appservice_login_user_id(identifier)?;
    let token = aa.require_access_token()?;
    // Ensure file-backed appservice registrations are loaded before token lookup.
    crate::appservices().await;
    let appservice = crate::appservice::find_from_token(token)
        .await?
        .ok_or_else(|| MatrixError::forbidden("Invalid application service token.", None))?;

    ensure_appservice_can_login_as(&appservice, &user_id)?;
    Ok(user_id)
}

fn appservice_login_user_id(identifier: &UserIdentifier) -> Result<OwnedUserId, MatrixError> {
    let username = if let UserIdentifier::Matrix(user_id) = identifier {
        user_id.user.to_lowercase()
    } else {
        return Err(MatrixError::forbidden("Bad login type.", None));
    };
    UserId::parse_with_server_name(username, &config::get().server_name)
        .map_err(|_| MatrixError::invalid_username("Username is invalid."))
}

fn ensure_appservice_can_login_as(
    appservice: &crate::appservice::RegistrationInfo,
    user_id: &UserId,
) -> Result<(), MatrixError> {
    if appservice.is_user_match(user_id) {
        Ok(())
    } else {
        Err(MatrixError::forbidden(
            "User is not in appservice's namespace",
            None,
        ))
    }
}

#[derive(Serialize)]
struct DelegatedPasswordLoginRequest<'a> {
    username: &'a str,
    password: &'a str,
    device_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_device_display_name: Option<String>,
    refresh_token: bool,
}

#[derive(Deserialize)]
struct DelegatedPasswordLoginResponse {
    access_token: String,
    user_id: String,
    device_id: String,
    #[serde(default)]
    expires_in_ms: Option<u64>,
}

async fn delegated_password_login(
    da: &config::DelegatedAuthConfig,
    user_id: &UserId,
    password: &str,
    device_id: OwnedDeviceId,
    initial_device_display_name: Option<String>,
) -> JsonResult<LoginResBody> {
    let endpoint = da
        .password_login_endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.trim().is_empty())
        .ok_or_else(|| {
            MatrixError::forbidden(
                "Password login is not configured for delegated authentication.",
                None,
            )
        })?;
    let mas_secret = config::get()
        .admin
        .mas_secret
        .as_deref()
        .ok_or_else(|| MatrixError::unknown("admin.mas_secret not configured"))?;

    let request = DelegatedPasswordLoginRequest {
        username: user_id.as_str(),
        password,
        device_id: device_id.as_str(),
        initial_device_display_name,
        // Palpo's Matrix /refresh endpoint only validates locally stored
        // refresh tokens. Do not request delegated refresh tokens until that
        // endpoint can proxy delegated refreshes as well.
        refresh_token: false,
    };

    let client = crate::sending::default_client();
    let response = client
        .post(endpoint)
        .bearer_auth(mas_secret)
        .json(&request)
        .send()
        .await
        .map_err(|e| MatrixError::unknown(format!("Delegated password login failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(MatrixError::forbidden("Wrong username or password.", None).into());
        }
        return Err(MatrixError::unknown(format!(
            "Delegated password login failed (status={status}): {body}"
        ))
        .into());
    }

    let delegated = response
        .json::<DelegatedPasswordLoginResponse>()
        .await
        .map_err(|e| {
            MatrixError::unknown(format!(
                "Delegated password login returned invalid JSON: {e}"
            ))
        })?;
    let delegated_user_id = UserId::parse(delegated.user_id)
        .map_err(|_| MatrixError::unknown("Delegated password login returned invalid user_id"))?;
    if delegated_user_id != user_id {
        return Err(
            MatrixError::unknown("Delegated password login returned a mismatched user_id").into(),
        );
    }
    let delegated_device_id: OwnedDeviceId = delegated.device_id.into();
    if delegated_device_id.as_str() != device_id.as_str() {
        return Err(MatrixError::unknown(
            "Delegated password login returned a mismatched device_id",
        )
        .into());
    }
    verify_delegated_login_token(&delegated.access_token, &delegated_user_id, &device_id).await?;
    data::user::device::delete_access_tokens(&delegated_user_id, &device_id).await?;
    data::user::device::delete_refresh_tokens(&delegated_user_id, &device_id).await?;

    tracing::info!(
        "{} logged in with delegated password auth",
        delegated_user_id
    );

    json_ok(LoginResBody {
        user_id: delegated_user_id,
        access_token: delegated.access_token,
        device_id,
        well_known: None,
        refresh_token: None,
        expires_in: delegated.expires_in_ms.map(Duration::from_millis),
    })
}

async fn verify_delegated_login_token(
    access_token: &str,
    expected_user_id: &UserId,
    expected_device_id: &DeviceId,
) -> AppResult<()> {
    let introspection = hoops::introspection::introspect_token(access_token).await?;
    if !introspection.active {
        return Err(
            MatrixError::unknown("Delegated password login returned an inactive token").into(),
        );
    }

    let username = introspection.username.as_deref().ok_or_else(|| {
        MatrixError::unknown("Delegated password login token is missing username")
    })?;
    let token_user_id = UserId::parse_with_server_name(username, &config::get().server_name)
        .map_err(|_| MatrixError::unknown("Delegated password login token has invalid username"))?;
    if token_user_id != expected_user_id {
        return Err(MatrixError::unknown(
            "Delegated password login token resolved to a mismatched user_id",
        )
        .into());
    }

    let token_device_id = introspection
        .device_id
        .or_else(|| {
            introspection
                .scope
                .as_deref()
                .and_then(hoops::introspection::device_id_from_scope)
        })
        .ok_or_else(|| {
            MatrixError::unknown("Delegated password login token is missing device_id")
        })?;
    if token_device_id.as_str() != expected_device_id.as_str() {
        return Err(MatrixError::unknown(
            "Delegated password login token resolved to a mismatched device_id",
        )
        .into());
    }

    let mut conn = connect().await?;
    let mut user = users::table
        .find(expected_user_id)
        .first::<DbUser>(&mut conn)
        .await
        .map_err(|_| MatrixError::unknown("Delegated password login user is not provisioned"))?;
    if user.is_guest {
        crate::data::user::set_guest(expected_user_id, false).await?;
        user.is_guest = false;
    }
    crate::user::ensure_account_usable(&user)?;

    user_devices::table
        .filter(user_devices::user_id.eq(expected_user_id))
        .filter(user_devices::device_id.eq(expected_device_id))
        .first::<DbUserDevice>(&mut conn)
        .await
        .map_err(|_| MatrixError::unknown("Delegated password login device is not provisioned"))?;

    Ok(())
}

/// # `POST /_matrix/client/v1/login/get_token`
///
/// Allows a logged-in user to get a short-lived token which can be used
/// to log in with the m.login.token flow.
///
/// <https://spec.matrix.org/v1.13/client-server-api/#post_matrixclientv1loginget_token>
#[endpoint]
async fn get_access_token(
    _aa: AuthArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<TokenResBody> {
    let conf = crate::config::get();
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();
    let device_id = authed.device_id();

    if !conf.login_via_existing_session {
        return Err(
            MatrixError::forbidden("login via an existing session is not enabled", None).into(),
        );
    }
    if conf.enabled_delegated_auth().is_some() {
        return Err(MatrixError::forbidden(
            "Login token issuance via password UIAA is disabled while delegated authentication is enabled.",
            None,
        )
        .into());
    }

    // This route SHOULD have UIA
    // TODO: How do we make only UIA sessions that have not been used before valid?
    let mut uiaa_info = UiaaInfo {
        flows: vec![AuthFlow {
            stages: vec![AuthType::Password],
        }],
        completed: Vec::new(),
        params: None,
        session: None,
        auth_error: None,
    };

    let payload = req.payload().await?;
    let body = serde_json::from_slice::<TokenReqBody>(payload);
    if let Ok(Some(auth)) = body.as_ref().map(|b| &b.auth) {
        let (worked, uiaa_info) =
            crate::uiaa::try_auth(sender_id, device_id, auth, &uiaa_info).await?;

        if !worked {
            return Err(AppError::Uiaa(uiaa_info));
        }
    } else if let Ok(json) = serde_json::from_slice::<CanonicalJsonValue>(payload) {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        crate::uiaa::create_session(sender_id, device_id, &uiaa_info, json).await?;
        return Err(AppError::Uiaa(uiaa_info));
    } else {
        return Err(MatrixError::not_json("No JSON body was sent when required.").into());
    }

    let login_token = utils::random_string(TOKEN_LENGTH);
    let expires_in = crate::user::create_login_token(sender_id, &login_token).await?;

    json_ok(TokenResBody {
        expires_in: Duration::from_millis(expires_in),
        login_token,
    })
}

/// #POST /_matrix/client/r0/logout
/// Log out the current device.
///
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
/// - With delegated auth: revokes the OAuth2 token at the auth provider
#[endpoint]
async fn logout(_aa: AuthArgs, req: &mut Request, depot: &mut Depot) -> EmptyResult {
    let Ok(authed) = depot.authed_info() else {
        return empty_ok();
    };

    // Delegated tokens are owned by the OIDC provider; local password/appservice
    // sessions are revoked only from Palpo's local device tables below.
    if authed.is_delegated_auth()
        && let Some(da) = config::get().enabled_delegated_auth()
        && let Some(token) = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        && let Err(e) = revoke_delegated_token(da, token).await
    {
        tracing::warn!("Failed to revoke delegated auth token: {e}");
    }

    user::remove_device(authed.user_id(), authed.device_id()).await?;
    empty_ok()
}

/// Call the OIDC provider's revocation endpoint to end the OAuth2 session.
async fn revoke_delegated_token(
    da: &config::DelegatedAuthConfig,
    access_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let issuer = da
        .issuer
        .as_deref()
        .ok_or("delegated auth issuer not configured")?;
    let mas_secret = config::get()
        .admin
        .mas_secret
        .as_deref()
        .ok_or("admin.mas_secret not configured")?;

    let revocation_url = format!("{}/oauth2/revoke", issuer.trim_end_matches('/'));

    let client = crate::sending::default_client();
    let response = client
        .post(&revocation_url)
        .bearer_auth(mas_secret)
        .form(&[("token", access_token), ("token_type_hint", "access_token")])
        .send()
        .await?;

    if !response.status().is_success() {
        tracing::warn!("Token revocation returned status: {}", response.status());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_appservice_info() -> crate::appservice::RegistrationInfo {
        use crate::core::appservice::{Namespace, Namespaces, Registration};

        Registration {
            id: "test".to_owned(),
            url: None,
            as_token: "as-token".to_owned(),
            hs_token: "hs-token".to_owned(),
            sender_localpart: "bridgebot".to_owned(),
            namespaces: Namespaces {
                users: vec![Namespace::new(
                    true,
                    r"^@bridge_.+:example\.com$".to_owned(),
                )],
                aliases: Vec::new(),
                rooms: Vec::new(),
            },
            rate_limited: None,
            protocols: None,
            receive_ephemeral: false,
            device_management: false,
        }
        .try_into()
        .unwrap()
    }

    #[test]
    fn delegated_auth_advertises_delegated_login_flows_without_password_exchange() {
        let flows = supported_login_flows(true, false);
        let flow_types = flows.iter().map(LoginType::login_type).collect::<Vec<_>>();

        assert_eq!(
            flow_types,
            vec!["m.login.application_service", "m.login.sso"]
        );
    }

    #[test]
    fn delegated_auth_advertises_password_when_exchange_is_configured() {
        let flows = supported_login_flows(true, true);
        let flow_types = flows.iter().map(LoginType::login_type).collect::<Vec<_>>();

        assert_eq!(
            flow_types,
            vec![
                "m.login.password",
                "m.login.application_service",
                "m.login.sso"
            ]
        );
    }

    #[test]
    fn legacy_auth_keeps_existing_login_flows() {
        let flows = supported_login_flows(false, false);
        let flow_types = flows.iter().map(LoginType::login_type).collect::<Vec<_>>();

        assert_eq!(
            flow_types,
            vec!["m.login.password", "m.login.application_service"]
        );
    }

    #[test]
    fn appservice_login_allows_namespaced_users() {
        let appservice = test_appservice_info();
        let user_id = UserId::parse("@bridge_alice:example.com").unwrap();

        assert!(ensure_appservice_can_login_as(&appservice, &user_id).is_ok());
    }

    #[test]
    fn appservice_login_allows_sender_localpart() {
        let appservice = test_appservice_info();
        let user_id = UserId::parse("@bridgebot:example.com").unwrap();

        assert!(ensure_appservice_can_login_as(&appservice, &user_id).is_ok());
    }

    #[test]
    fn appservice_login_rejects_users_outside_namespace() {
        let appservice = test_appservice_info();
        let user_id = UserId::parse("@alice:example.com").unwrap();

        assert!(ensure_appservice_can_login_as(&appservice, &user_id).is_err());
    }
}

/// #POST /_matrix/client/r0/logout/all
/// Log out all devices of this user.
///
/// - Invalidates all access tokens
/// - Deletes all device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets all to-device events
/// - Triggers device list updates
///
/// Note: This is equivalent to calling [`GET /_matrix/client/r0/logout`](fn.logout_route.html)
/// from each device of this user.
#[endpoint]
async fn logout_all(_aa: AuthArgs, depot: &mut Depot) -> EmptyResult {
    let Ok(authed) = depot.authed_info() else {
        return empty_ok();
    };

    crate::user::remove_all_devices(authed.user_id()).await?;

    empty_ok()
}

#[endpoint]
async fn refresh_access_token(
    _aa: AuthArgs,
    body: JsonBody<RefreshTokenReqBody>,
    depot: &mut Depot,
) -> JsonResult<RefreshTokenResBody> {
    let authed = depot.authed_info()?;
    let user_id = authed.user_id();
    let device_id = authed.device_id();
    crate::user::valid_refresh_token(user_id, device_id, &body.refresh_token).await?;

    let access_token = utils::random_string(TOKEN_LENGTH);
    let refresh_token = utils::random_string(TOKEN_LENGTH);
    let expires_at = UnixMillis::now().get() + crate::config::get().refresh_token_ttl;
    let ultimate_session_expires_at = UnixMillis::now().get() + crate::config::get().session_ttl;
    let refresh_token_id = data::user::device::set_refresh_token(
        user_id,
        device_id,
        &refresh_token,
        expires_at,
        ultimate_session_expires_at,
    )
    .await?;
    if data::user::device::is_device_exists(user_id, device_id).await? {
        data::user::device::set_access_token(
            user_id,
            device_id,
            &access_token,
            Some(refresh_token_id),
        )
        .await?;
    } else {
        return Err(MatrixError::not_found("Device not found.").into());
    }
    json_ok(RefreshTokenResBody {
        access_token,
        refresh_token: Some(refresh_token),
        expires_in_ms: Some(Duration::from_millis(expires_at - UnixMillis::now().get())),
    })
}

/// Extract redirectUrl from query string (Element sends camelCase `redirectUrl`).
fn get_redirect_url(req: &Request) -> Result<String, MatrixError> {
    req.query::<String>("redirectUrl")
        .or_else(|| req.query::<String>("redirect_url"))
        .ok_or_else(|| MatrixError::bad_json("Missing redirectUrl parameter"))
}

/// Build the authorization URL for the delegated auth issuer.
fn build_sso_redirect_url(redirect_url: &str) -> Result<String, MatrixError> {
    let conf = config::get();
    let da = conf
        .enabled_delegated_auth()
        .ok_or_else(|| MatrixError::not_found("SSO is not enabled on this server"))?;
    let issuer = da
        .issuer
        .as_deref()
        .ok_or_else(|| MatrixError::unknown("Delegated auth issuer not configured"))?;
    let client_id = da
        .client_id
        .as_deref()
        .ok_or_else(|| MatrixError::unknown("Delegated auth client_id not configured"))?;

    let state = utils::random_string(TOKEN_LENGTH);
    let authorize_url = format!("{}/authorize", issuer.trim_end_matches('/'));
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_url", redirect_url)
        .append_pair("scope", "openid urn:matrix:org.matrix.msc2967.client:api:*")
        .append_pair("state", &state)
        .finish();

    Ok(format!("{authorize_url}?{params}"))
}

#[endpoint]
async fn redirect(_aa: AuthArgs, req: &mut Request, res: &mut Response) -> AppResult<()> {
    let redirect_url = get_redirect_url(req)?;
    let auth_url = build_sso_redirect_url(&redirect_url)?;
    res.render(salvo::prelude::Redirect::found(auth_url));
    Ok(())
}

#[endpoint]
async fn provider_url(_aa: AuthArgs, req: &mut Request, res: &mut Response) -> AppResult<()> {
    let redirect_url = get_redirect_url(req)?;
    let auth_url = build_sso_redirect_url(&redirect_url)?;
    res.render(salvo::prelude::Redirect::found(auth_url));
    Ok(())
}
