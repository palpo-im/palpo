use std::time::Duration;

use diesel::prelude::*;
use palpo_data::user::set_display_name;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::client::session::*;
use crate::core::client::uiaa::{AuthFlow, AuthType, UiaaInfo, UserIdentifier};
use crate::core::identifiers::*;
use crate::core::serde::CanonicalJsonValue;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::{DbUser, NewDbUser};
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
                    .push(Router::with_path("{idpId}").get(provider_url)),
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
    let conf = config::get();
    let flows = if conf.enabled_delegated_auth().is_some() {
        // MSC3861 delegated auth — SSO is the only flow.
        vec![LoginType::Sso(
            crate::core::client::session::SsoLoginType::new(),
        )]
    } else if let Some(oidc_cfg) = conf.enabled_oidc() {
        // [oidc] providers configured — advertise m.login.sso with the
        // provider list, keeping password + appservice as secondary flows
        // so existing token-gated registration paths still work.
        let identity_providers: Vec<IdentityProvider> = oidc_cfg
            .providers
            .iter()
            .map(|(id, pc)| {
                IdentityProvider::new(
                    id.clone(),
                    pc.display_name.clone().unwrap_or_else(|| id.clone()),
                )
            })
            .collect();
        vec![
            LoginType::Sso(SsoLoginType { identity_providers }),
            LoginType::password(),
            LoginType::appservice(),
        ]
    } else {
        vec![LoginType::password(), LoginType::appservice()]
    };
    Ok(Json(LoginTypesResBody::new(flows)))
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
    body: JsonBody<LoginReqBody>,
    req: &mut Request,
    res: &mut Response,
) -> JsonResult<LoginResBody> {
    if config::get().enabled_delegated_auth().is_some() {
        return Err(MatrixError::forbidden(
            "This server uses delegated authentication. Use the OIDC provider to log in.",
            None,
        )
        .into());
    }

    // Validate login method
    // TODO: Other login methods
    let user_id = match &body.login_info {
        LoginInfo::Password(Password {
            identifier,
            password,
        }) => {
            let username = if let UserIdentifier::UserIdOrLocalpart(user_id) = identifier {
                user_id.to_lowercase()
            } else {
                warn!("Bad login type: {:?}", &body.login_info);
                return Err(MatrixError::forbidden("Bad login type.", None).into());
            };
            let user_id = UserId::parse_with_server_name(username, &config::get().server_name)
                .map_err(|_| MatrixError::invalid_username("Username is invalid."))?;

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
            let Ok(user) = data::user::get_user(&user_id) else {
                return Err(MatrixError::forbidden("User not found.", None).into());
            };
            if let Err(_e) = user::verify_password(&user, password) {
                res.status_code(StatusCode::FORBIDDEN); //for complement testing: TestLogin/parallel/POST_/login_wrong_password_is_rejected
                return Err(MatrixError::forbidden("Wrong username or password.", None).into());
            }
            // }

            user_id
        }
        LoginInfo::Token(Token { token }) => {
            if !crate::config::get().login_via_existing_session {
                return Err(MatrixError::unknown("Token login is not enabled.").into());
            }
            user::take_login_token(token)?
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

            if !data::user::user_exists(&user_id)? {
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
                    .get_result::<DbUser>(&mut connect()?)?;

                // Set initial user profile
                if let Err(e) = set_display_name(&user.id, user.id.localpart()) {
                    tracing::warn!("failed to set profile for new user (non-fatal): {}", e);
                }
            }
            user_id
        }
        LoginInfo::Appservice(Appservice { identifier }) => {
            let username = if let UserIdentifier::UserIdOrLocalpart(user_id) = identifier {
                user_id.to_lowercase()
            } else {
                return Err(MatrixError::forbidden("Bad login type.", None).into());
            };
            UserId::parse_with_server_name(username, &config::get().server_name)
                .map_err(|_| MatrixError::invalid_username("Username is invalid."))?
        }
        _ => {
            warn!("Unsupported or unknown login type: {:?}", &body.login_info);
            return Err(MatrixError::unknown("Unsupported login type.").into());
        }
    };

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
        )?;
        (Some(refresh_token), Some(refresh_token_id))
    } else {
        (None, None)
    };

    // Determine if device_id was provided and exists in the db for this user
    if data::user::device::is_device_exists(&user_id, &device_id)? {
        data::user::device::set_access_token(
            &user_id,
            &device_id,
            &access_token,
            refresh_token_id,
        )?;
    } else {
        data::user::device::create_device(
            &user_id,
            &device_id,
            &access_token,
            body.initial_device_display_name.clone(),
            Some(req.remote_addr().to_string()),
        )?;
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
        let (worked, uiaa_info) = crate::uiaa::try_auth(sender_id, device_id, auth, &uiaa_info)?;

        if !worked {
            return Err(AppError::Uiaa(uiaa_info));
        }
    } else if let Ok(json) = serde_json::from_slice::<CanonicalJsonValue>(payload) {
        uiaa_info.session = Some(utils::random_string(SESSION_ID_LENGTH));
        let _ = crate::uiaa::create_session(sender_id, device_id, &uiaa_info, json);
        return Err(AppError::Uiaa(uiaa_info));
    } else {
        return Err(MatrixError::not_json("No JSON body was sent when required.").into());
    }

    let login_token = utils::random_string(TOKEN_LENGTH);
    let expires_in = crate::user::create_login_token(sender_id, &login_token)?;

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

    // When using delegated auth, revoke the access token at the OIDC provider
    // BEFORE removing the local device. If upstream revocation fails, we
    // surface that to the caller and leave the local device intact so the
    // client can retry — otherwise the client believes it logged out while
    // the upstream OAuth2 session is still live and the token still valid
    // for any other resource server that trusts MAS-issued tokens.
    if let Some(da) = config::get().enabled_delegated_auth()
        && let Some(token) = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        && let Err(e) = revoke_delegated_token(da, token).await
    {
        tracing::error!("Failed to revoke delegated auth token: {e}");
        return Err(MatrixError::unknown(format!(
            "Upstream token revocation failed; please retry: {e}"
        ))
        .into());
    }

    data::user::device::remove_device(authed.user_id(), authed.device_id())?;
    empty_ok()
}

/// Call the OIDC provider's revocation endpoint to end the OAuth2 session.
///
/// Authenticates per RFC 6749 §2.3.1: prefers HTTP Basic with
/// `client_id:client_secret` when both are configured (RFC 7009 §2.1's
/// expected method for confidential clients). Falls back to bearer with
/// `admin.mas_secret` for back-compat with deployments that register Palpo
/// as a public client and rely on the homeserver-admin shared bearer.
async fn revoke_delegated_token(
    da: &config::DelegatedAuthConfig,
    access_token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let issuer = da
        .issuer
        .as_deref()
        .ok_or("delegated auth issuer not configured")?;

    let revocation_url = format!("{}/oauth2/revoke", issuer.trim_end_matches('/'));

    let client = crate::sending::default_client();
    let mut request = client.post(&revocation_url);
    let mut form_params: Vec<(&str, &str)> = vec![
        ("token", access_token),
        ("token_type_hint", "access_token"),
    ];
    match (da.client_id.as_deref(), da.client_secret.as_deref()) {
        (Some(id), Some(secret)) => {
            request = request.basic_auth(id, Some(secret));
        }
        (Some(id), None) => {
            form_params.push(("client_id", id));
        }
        _ => {
            let mas_secret = config::get()
                .admin
                .mas_secret
                .as_deref()
                .ok_or("admin.mas_secret not configured")?;
            request = request.bearer_auth(mas_secret);
        }
    }

    let response = request.form(&form_params).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(format!("token revocation returned status {status}").into());
    }

    Ok(())
}

/// #POST /_matrix/client/r0/logout/all
/// Log out all devices of this user.
///
/// - Invalidates all access tokens
/// - Deletes all device metadata (device id, device display name, last seen ip, last seen ts)
/// - Forgets all to-device events
/// - Triggers device list updates
/// - With delegated auth: revokes the current OAuth2 token at the auth
///   provider. Tokens belonging to OTHER devices cannot be revoked from
///   here — under MSC3861, Palpo doesn't store the upstream tokens; only
///   the device that initiated this request carries one. Users who want a
///   true "sign out everywhere" should use MAS's account UI session list,
///   which has the full upstream session inventory. Tracked as a TODO
///   upstream: needs MAS admin API for kill-sessions-by-user.
///
/// Note: This is equivalent to calling [`GET /_matrix/client/r0/logout`](fn.logout_route.html)
/// from each device of this user — except for the upstream-revocation gap
/// noted above.
#[endpoint]
async fn logout_all(_aa: AuthArgs, req: &mut Request, depot: &mut Depot) -> EmptyResult {
    let Ok(authed) = depot.authed_info() else {
        return empty_ok();
    };

    // Best-effort revocation of the current device's OAuth2 token at the
    // upstream provider. Unlike `logout`, we proceed with the full local
    // cleanup even on failure — the user explicitly asked to log out
    // everywhere, so we honour that locally regardless of upstream state.
    // Failure is still raised to the caller after local cleanup so the
    // client can show a "MAS session may still be live, visit account UI"
    // message rather than a misleading green checkmark.
    let mut revocation_error: Option<String> = None;
    if let Some(da) = config::get().enabled_delegated_auth()
        && let Some(token) = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        && let Err(e) = revoke_delegated_token(da, token).await
    {
        tracing::error!("Failed to revoke delegated auth token in logout_all: {e}");
        revocation_error = Some(e.to_string());
    }

    data::user::remove_all_devices(authed.user_id())?;

    if let Some(e) = revocation_error {
        return Err(MatrixError::unknown(format!(
            "Local sessions cleared but upstream token revocation failed; \
             visit your account-management UI to verify the upstream session \
             is also signed out: {e}"
        ))
        .into());
    }

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
    crate::user::valid_refresh_token(user_id, device_id, &body.refresh_token)?;

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
    )?;
    if data::user::device::is_device_exists(user_id, device_id)? {
        data::user::device::set_access_token(
            user_id,
            device_id,
            &access_token,
            Some(refresh_token_id),
        )?;
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

/// Reject any `redirectUrl` not matching a configured whitelist entry.
/// Defense against login-token theft via attacker-crafted redirectUrl.
///
/// Match semantics: the input and each whitelist entry are parsed as URLs;
/// **origins must match exactly** (scheme + host + port) and the input path
/// must equal or be below the entry's path. Both sides must be `https://`.
/// Whitespace-only and unparsable whitelist entries are skipped (logged).
/// Raw byte-prefix matching is intentionally not used — it accepts hostile
/// inputs like `https://app.example.io.evil.com/` against an entry of
/// `https://app.example.io` (no trailing slash) and is wildcarded by an
/// empty-string entry.
///
/// Applies only to the `[oidc]` path. The `[delegated_auth]` path does not
/// emit tokens to the supplied URL (the IdP issuer handles the return), so
/// this validator is a no-op there and relies on the IdP's own validation
/// of client redirect URIs.
///
/// Made `pub(crate)` so the oidc.rs handler can also defense-in-depth check
/// the cookie contents it reads on callback.
pub(crate) fn validate_sso_redirect_url(input: &str) -> Result<(), MatrixError> {
    let conf = config::get();
    let Some(oidc_cfg) = conf.enabled_oidc() else {
        // Non-OIDC path (delegated_auth or unconfigured): skip.
        return Ok(());
    };
    match_sso_redirect_url(input, &oidc_cfg.sso_client_whitelist)
}

/// Pure helper for `validate_sso_redirect_url`. Extracted so the whitelist
/// matching logic can be unit-tested without touching the global config.
fn match_sso_redirect_url(input: &str, whitelist: &[String]) -> Result<(), MatrixError> {
    if whitelist.is_empty() {
        return Err(MatrixError::forbidden(
            "SSO redirect rejected: no oidc.sso_client_whitelist configured",
            None,
        ));
    }

    let target = url::Url::parse(input)
        .map_err(|_| MatrixError::forbidden("redirectUrl is not a valid URL", None))?;
    if target.scheme() != "https" {
        return Err(MatrixError::forbidden(
            "redirectUrl must use https scheme",
            None,
        ));
    }
    if target.host_str().is_none() {
        return Err(MatrixError::forbidden(
            "redirectUrl is missing a host",
            None,
        ));
    }

    for entry in whitelist {
        if entry.trim().is_empty() {
            tracing::warn!("Skipping empty sso_client_whitelist entry");
            continue;
        }
        let allowed = match url::Url::parse(entry) {
            Ok(u) => u,
            Err(_) => {
                tracing::warn!("Skipping malformed sso_client_whitelist entry: {entry:?}");
                continue;
            }
        };
        if allowed.scheme() != "https" {
            tracing::warn!("Skipping non-https sso_client_whitelist entry: {entry:?}");
            continue;
        }
        if target.origin() != allowed.origin() {
            continue;
        }
        let allowed_path = allowed.path();
        let target_path = target.path();
        // Equal, or target_path begins with allowed_path followed by a '/' boundary.
        // This prevents `/admin` from allowing `/administrator`.
        if target_path == allowed_path
            || target_path.starts_with(allowed_path)
                && (allowed_path.ends_with('/')
                    || target_path[allowed_path.len()..].starts_with('/'))
        {
            return Ok(());
        }
    }
    Err(MatrixError::forbidden(
        "SSO redirect rejected: redirectUrl not in oidc.sso_client_whitelist",
        None,
    ))
}

#[cfg(test)]
mod sso_whitelist_tests {
    use super::match_sso_redirect_url;

    fn wl(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn allows_exact_origin_match() {
        let w = wl(&["https://app.element.io/"]);
        assert!(match_sso_redirect_url("https://app.element.io/", &w).is_ok());
        assert!(match_sso_redirect_url("https://app.element.io/path", &w).is_ok());
        assert!(match_sso_redirect_url("https://app.element.io/?x=1", &w).is_ok());
    }

    #[test]
    fn rejects_sibling_host_with_missing_trailing_slash() {
        // The pre-fix prefix-match accepted this for entry "https://app.element.io".
        let w = wl(&["https://app.element.io"]);
        assert!(match_sso_redirect_url("https://app.element.io.evil.com/", &w).is_err());
    }

    #[test]
    fn rejects_userinfo_smuggling() {
        let w = wl(&["https://app.element.io/"]);
        assert!(match_sso_redirect_url("https://app.element.io@evil.com/", &w).is_err());
    }

    #[test]
    fn rejects_different_port() {
        let w = wl(&["https://app.element.io/"]);
        assert!(match_sso_redirect_url("https://app.element.io:8443/", &w).is_err());
    }

    #[test]
    fn rejects_http_scheme_input() {
        let w = wl(&["https://app.element.io/"]);
        assert!(match_sso_redirect_url("http://app.element.io/", &w).is_err());
    }

    #[test]
    fn rejects_javascript_scheme_input() {
        let w = wl(&["https://app.element.io/"]);
        assert!(match_sso_redirect_url("javascript:alert(1)", &w).is_err());
    }

    #[test]
    fn empty_string_entry_is_not_a_wildcard() {
        // The pre-fix prefix-match made [""] match every URL.
        let w = wl(&["", "https://app.element.io/"]);
        assert!(match_sso_redirect_url("https://app.element.io/", &w).is_ok());
        assert!(match_sso_redirect_url("https://evil.com/", &w).is_err());
    }

    #[test]
    fn skips_non_https_entry() {
        let w = wl(&["http://app.element.io/", "https://app.element.io/"]);
        assert!(match_sso_redirect_url("https://app.element.io/", &w).is_ok());
        // The http entry must not be promoted to allow http inputs.
        assert!(match_sso_redirect_url("http://app.element.io/", &w).is_err());
    }

    #[test]
    fn empty_whitelist_rejects_all() {
        let w = wl(&[]);
        assert!(match_sso_redirect_url("https://app.element.io/", &w).is_err());
    }

    #[test]
    fn path_prefix_respects_segment_boundary() {
        let w = wl(&["https://app.element.io/admin"]);
        assert!(match_sso_redirect_url("https://app.element.io/admin", &w).is_ok());
        assert!(match_sso_redirect_url("https://app.element.io/admin/x", &w).is_ok());
        // Pre-fix would have allowed this; new check rejects.
        assert!(match_sso_redirect_url("https://app.element.io/administrator", &w).is_err());
    }

    #[test]
    fn host_match_is_case_insensitive_via_url_normalization() {
        // url::Url lowercases hosts on parse, so case differences in input
        // and entry are handled identically.
        let w = wl(&["https://APP.Element.IO/"]);
        assert!(match_sso_redirect_url("https://app.element.io/", &w).is_ok());
    }
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

/// Decide which backend to redirect to based on which OIDC path is enabled.
/// `[oidc]` (multi-provider OAuth) takes precedence over `[delegated_auth]`
/// (MSC3861). For `[oidc]`, we bounce through our own `/oidc/auth` so it can
/// set the CSRF/PKCE cookie and stash the client's redirectUrl. For
/// `[delegated_auth]`, we build the IdP authorize URL directly.
fn build_auth_redirect_url(
    idp_id: Option<String>,
    client_redirect_url: &str,
) -> Result<String, MatrixError> {
    let conf = config::get();
    if conf.enabled_oidc().is_some() {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        if let Some(id) = idp_id {
            serializer.append_pair("provider", &id);
        }
        serializer.append_pair("redirectUrl", client_redirect_url);
        return Ok(format!("/_matrix/client/oidc/auth?{}", serializer.finish()));
    }
    build_sso_redirect_url(client_redirect_url)
}

#[endpoint]
async fn redirect(_aa: AuthArgs, req: &mut Request, res: &mut Response) -> AppResult<()> {
    let redirect_url = get_redirect_url(req)?;
    validate_sso_redirect_url(&redirect_url)?;
    let auth_url = build_auth_redirect_url(None, &redirect_url)?;
    res.render(salvo::prelude::Redirect::found(auth_url));
    Ok(())
}

#[endpoint]
async fn provider_url(_aa: AuthArgs, req: &mut Request, res: &mut Response) -> AppResult<()> {
    let redirect_url = get_redirect_url(req)?;
    validate_sso_redirect_url(&redirect_url)?;
    let idp_id = req.param::<String>("idpId");
    let auth_url = build_auth_redirect_url(idp_id, &redirect_url)?;
    res.render(salvo::prelude::Redirect::found(auth_url));
    Ok(())
}
