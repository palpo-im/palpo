//! OAuth2 Authorization Server endpoints (MSC3861)
//!
//! Implements the OAuth Broker pattern: Palpo acts as an Authorization Server
//! to Element X, while delegating actual authentication to configured external
//! OAuth providers (GitHub, Google, etc.).

use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::time::Duration;
use diesel::prelude::*;
use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use url::form_urlencoded;

use crate::config::{self, OidcProviderConfig};
use crate::core::{MatrixError, OwnedDeviceId, UnixMillis};
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::DbUser;
use crate::routing::client::oauth2::{AuthMetadataResponse, build_auth_metadata};
use crate::{AppResult, JsonResult, data, json_ok, utils};

// =================== ROUTES ===================

pub fn router() -> salvo::Router {
    salvo::Router::with_path("oauth2")
        .push(salvo::Router::with_path("register").post(client_register))
        .push(salvo::Router::with_path("authorize").get(authorize))
        .push(salvo::Router::with_path("provider_callback").get(provider_callback))
        .push(salvo::Router::with_path("token").post(token_exchange))
        .push(salvo::Router::with_path("revoke").post(token_revoke))
}

// =================== TYPES ===================

/// OAuth2 session for linking Element X ↔ Provider flows
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OAuth2Session {
    // Element X leg
    ex_client_id: String,
    ex_redirect_uri: String,
    ex_state: String,
    ex_code_challenge: String,
    ex_scope: String,
    // Provider leg
    provider_name: String,
    provider_state: String,
    provider_code_verifier: Option<String>,
    created_at: u64,
}

#[derive(Debug, Serialize, ToSchema)]
struct ClientRegistrationResponse {
    client_id: String,
    client_id_issued_at: i64,
    client_id_expires_at: i64,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: String,
    response_types: Vec<String>,
    grant_types: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ClientRegistrationRequest {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(default = "default_auth_method")]
    token_endpoint_auth_method: String,
    #[serde(default = "default_response_types")]
    response_types: Vec<String>,
    #[serde(default = "default_grant_types")]
    grant_types: Vec<String>,
    #[serde(default = "default_app_type")]
    application_type: String,
}

fn default_auth_method() -> String {
    "none".into()
}
fn default_response_types() -> Vec<String> {
    vec!["code".into()]
}
fn default_grant_types() -> Vec<String> {
    vec!["authorization_code".into(), "refresh_token".into()]
}
fn default_app_type() -> String {
    "native".into()
}

#[derive(Debug, Serialize, ToSchema)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: Option<i64>,
    scope: String,
}

#[derive(Debug, Serialize)]
struct OAuth2Error {
    error: String,
    error_description: String,
}

// =================== ENDPOINTS ===================

/// GET /.well-known/openid-configuration
///
/// RFC 8414 Authorization Server Metadata discovery.
#[endpoint]
pub async fn openid_configuration() -> JsonResult<AuthMetadataResponse> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    if !oidc.enable_auth_server {
        return Err(MatrixError::not_found("Authorization server not enabled").into());
    }

    let base = conf.well_known_client();
    let base = base.trim_end_matches('/');
    json_ok(build_auth_metadata(base))
}

/// POST /oauth2/register
///
/// Dynamic Client Registration (RFC 7591).
/// Element X registers itself on first use.
#[endpoint]
async fn client_register(body: JsonBody<ClientRegistrationRequest>) -> JsonResult<ClientRegistrationResponse> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    if !oidc.enable_auth_server {
        return Err(MatrixError::not_found("Authorization server not enabled").into());
    }

    if body.redirect_uris.is_empty() {
        return Err(MatrixError::invalid_param("redirect_uris is required and must not be empty").into());
    }

    let client_id = format!("palpo_client_{}", utils::random_string(32));
    let now = UnixMillis::now().get() as i64;

    let redirect_uris_json = serde_json::to_string(&body.redirect_uris)
        .map_err(|e| MatrixError::unknown(format!("Failed to serialize redirect_uris: {}", e)))?;
    let grant_types_json = serde_json::to_string(&body.grant_types)
        .map_err(|e| MatrixError::unknown(format!("Failed to serialize grant_types: {}", e)))?;
    let response_types_json = serde_json::to_string(&body.response_types)
        .map_err(|e| MatrixError::unknown(format!("Failed to serialize response_types: {}", e)))?;

    diesel::insert_into(oauth_clients::table)
        .values((
            oauth_clients::client_id.eq(&client_id),
            oauth_clients::client_name.eq(&body.client_name),
            oauth_clients::redirect_uris.eq(&redirect_uris_json),
            oauth_clients::token_endpoint_auth_method.eq(&body.token_endpoint_auth_method),
            oauth_clients::grant_types.eq(&grant_types_json),
            oauth_clients::response_types.eq(&response_types_json),
            oauth_clients::application_type.eq(&body.application_type),
            oauth_clients::created_at.eq(now),
        ))
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to register client: {}", e)))?;

    tracing::info!("Registered new OAuth2 client: {}", client_id);

    json_ok(ClientRegistrationResponse {
        client_id,
        client_id_issued_at: now / 1000, // seconds
        client_id_expires_at: 0,         // never expires
        redirect_uris: body.redirect_uris.clone(),
        token_endpoint_auth_method: body.token_endpoint_auth_method.clone(),
        response_types: body.response_types.clone(),
        grant_types: body.grant_types.clone(),
    })
}

/// GET /oauth2/authorize
///
/// Authorization endpoint. Element X opens this in system browser.
/// Validates the request, then redirects to the configured OAuth provider.
#[endpoint]
async fn authorize(req: &mut Request, res: &mut Response) -> AppResult<()> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    if !oidc.enable_auth_server {
        return Err(MatrixError::not_found("Authorization server not enabled").into());
    }

    // Extract Element X's request parameters
    let client_id = req
        .query::<String>("client_id")
        .ok_or_else(|| MatrixError::invalid_param("Missing client_id"))?;
    let redirect_uri = req
        .query::<String>("redirect_uri")
        .ok_or_else(|| MatrixError::invalid_param("Missing redirect_uri"))?;
    let response_type = req
        .query::<String>("response_type")
        .ok_or_else(|| MatrixError::invalid_param("Missing response_type"))?;
    let state = req
        .query::<String>("state")
        .ok_or_else(|| MatrixError::invalid_param("Missing state"))?;
    let code_challenge = req
        .query::<String>("code_challenge")
        .ok_or_else(|| MatrixError::invalid_param("Missing code_challenge"))?;
    let code_challenge_method = req
        .query::<String>("code_challenge_method")
        .unwrap_or_else(|| "S256".into());
    let scope = req
        .query::<String>("scope")
        .unwrap_or_else(|| "urn:matrix:org.matrix.msc2967.client:api:*".into());

    // Validate response_type
    if response_type != "code" {
        return Err(MatrixError::invalid_param("response_type must be 'code'").into());
    }

    // Validate code_challenge_method
    if code_challenge_method != "S256" {
        return Err(MatrixError::invalid_param("code_challenge_method must be 'S256'").into());
    }

    // Validate client_id exists
    let client = oauth_clients::table
        .filter(oauth_clients::client_id.eq(&client_id))
        .first::<(String, Option<String>, String, String, String, String, Option<String>, Option<i64>, i64)>(&mut connect()?)
        .optional()
        .map_err(|e| MatrixError::unknown(format!("DB error: {}", e)))?;

    let client = client.ok_or_else(|| MatrixError::invalid_param("Unknown client_id"))?;

    // Validate redirect_uri matches registered URIs (exact string match)
    let registered_uris: Vec<String> = serde_json::from_str(&client.2)
        .map_err(|_| MatrixError::unknown("Invalid registered redirect_uris"))?;

    if !registered_uris.iter().any(|uri| uri == &redirect_uri) {
        return Err(MatrixError::invalid_param("redirect_uri does not match registered URIs").into());
    }

    // Update last_used_at
    let _ = diesel::update(oauth_clients::table.filter(oauth_clients::client_id.eq(&client_id)))
        .set(oauth_clients::last_used_at.eq(Some(UnixMillis::now().get() as i64)))
        .execute(&mut connect()?);

    // Determine which provider to use (default or first configured)
    let provider_name = oidc
        .default_provider
        .clone()
        .or_else(|| oidc.providers.keys().next().cloned())
        .ok_or_else(|| MatrixError::unknown("No OIDC provider configured"))?;

    let provider_config = oidc.providers.get(&provider_name).ok_or_else(|| {
        MatrixError::unknown(format!("Provider '{}' not found", provider_name))
    })?;

    // Discover provider endpoints
    let provider_info = super::client::oidc::discover_provider_endpoints(provider_config).await?;

    // Generate provider-leg security tokens
    let provider_state = utils::random_string(32);
    let (provider_code_verifier, provider_code_challenge) = if oidc.enable_pkce {
        let (v, c) = generate_pkce();
        (Some(v), Some(c))
    } else {
        (None, None)
    };

    // Store BOTH legs in session cookie
    let session = OAuth2Session {
        ex_client_id: client_id,
        ex_redirect_uri: redirect_uri,
        ex_state: state,
        ex_code_challenge: code_challenge,
        ex_scope: scope,
        provider_name: provider_name.clone(),
        provider_state: provider_state.clone(),
        provider_code_verifier,
        created_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    let session_data = serde_json::to_string(&session)
        .map_err(|e| MatrixError::unknown(format!("Failed to serialize session: {}", e)))?;

    let is_production = !cfg!(debug_assertions);
    res.add_cookie(
        salvo::http::cookie::Cookie::build(("oauth2_session", session_data))
            .http_only(true)
            .secure(is_production)
            .same_site(salvo::http::cookie::SameSite::Lax)
            .max_age(Duration::seconds(oidc.session_timeout as i64))
            .path("/")
            .build(),
    );

    // Build provider authorization URL (reusing oidc.rs pattern)
    let mut auth_url = url::Url::parse(&provider_info.authorization_endpoint)
        .map_err(|e| MatrixError::unknown(format!("Invalid auth endpoint: {}", e)))?;

    // The provider callback goes to our /oauth2/provider_callback
    let provider_callback_url = format!("{}/oauth2/provider_callback", conf.well_known_client().trim_end_matches('/'));

    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("client_id", &provider_config.client_id)
            .append_pair("redirect_uri", &provider_callback_url)
            .append_pair("response_type", "code")
            .append_pair("state", &provider_state)
            .append_pair("scope", &provider_config.scopes.join(" "));

        if let Some(challenge) = &provider_code_challenge {
            q.append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
        }

        for (key, value) in &provider_config.additional_params {
            q.append_pair(key, value);
        }
    }

    tracing::info!("OAuth2 authorize: redirecting to provider '{}'", provider_name);
    res.render(Redirect::found(auth_url.to_string()));
    Ok(())
}

/// GET /oauth2/provider_callback
///
/// Handles the callback from the external OAuth provider.
/// Creates/retrieves Matrix user, generates Palpo auth code,
/// redirects back to Element X.
#[endpoint]
async fn provider_callback(req: &mut Request, res: &mut Response) -> AppResult<()> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    // Handle provider errors
    if let Some(error) = req.query::<String>("error") {
        let desc = req.query::<String>("error_description").unwrap_or_default();
        tracing::warn!("Provider returned error: {} - {}", error, desc);
        return Err(MatrixError::forbidden(format!("Provider auth failed: {}", desc), None).into());
    }

    // Extract provider callback params
    let code = req
        .query::<String>("code")
        .ok_or_else(|| MatrixError::invalid_param("Missing code"))?;
    let provider_state = req
        .query::<String>("state")
        .ok_or_else(|| MatrixError::invalid_param("Missing state"))?;

    // Restore session
    let session_cookie = req
        .cookie("oauth2_session")
        .ok_or_else(|| MatrixError::unauthorized("OAuth2 session not found or expired"))?;

    let session: OAuth2Session = serde_json::from_str(session_cookie.value())
        .map_err(|e| MatrixError::unauthorized(format!("Invalid session: {}", e)))?;

    // Validate provider state (CSRF)
    if provider_state != session.provider_state {
        return Err(MatrixError::forbidden("State mismatch", None).into());
    }

    // Check session timeout
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    if now > session.created_at + oidc.session_timeout {
        return Err(MatrixError::unauthorized("Session expired").into());
    }

    // Get provider config
    let provider_config = oidc.providers.get(&session.provider_name).ok_or_else(|| {
        MatrixError::unknown(format!("Provider '{}' no longer configured", session.provider_name))
    })?;

    let provider_info = super::client::oidc::discover_provider_endpoints(provider_config).await?;

    // The redirect_uri for the provider leg points to /oauth2/provider_callback
    let provider_callback_url = format!("{}/oauth2/provider_callback", conf.well_known_client().trim_end_matches('/'));

    // Exchange provider code for tokens
    let token_response = exchange_provider_code(
        &code,
        provider_config,
        &provider_info,
        &provider_callback_url,
        session.provider_code_verifier.as_deref(),
    )
    .await?;

    // Fetch user info from provider
    let user_info = super::client::oidc::get_user_info_from_provider(
        &token_response.access_token,
        &provider_info,
        provider_config,
    )
    .await?;

    // Validate user info
    super::client::oidc::validate_user_info(&user_info, oidc)?;

    // Generate Matrix user ID and create/get user
    let matrix_user_id = super::client::oidc::generate_matrix_user_id(&user_info, oidc, conf.server_name.as_str())?;
    let display_name = super::client::oidc::generate_display_name(&user_info, provider_config);
    let _user = super::client::oidc::create_or_get_user(&matrix_user_id, &display_name, &user_info, oidc).await?;

    // Generate Palpo authorization code (>= 256 bits entropy)
    let palpo_code = utils::random_string(43);
    let now_ms = UnixMillis::now().get() as i64;
    let expires_at = now_ms + 300_000; // 5 minutes

    // Store authorization code
    diesel::insert_into(oauth_authorization_codes::table)
        .values((
            oauth_authorization_codes::code.eq(&palpo_code),
            oauth_authorization_codes::client_id.eq(&session.ex_client_id),
            oauth_authorization_codes::user_id.eq(&matrix_user_id),
            oauth_authorization_codes::redirect_uri.eq(&session.ex_redirect_uri),
            oauth_authorization_codes::code_challenge.eq(&session.ex_code_challenge),
            oauth_authorization_codes::code_challenge_method.eq("S256"),
            oauth_authorization_codes::scope.eq(&session.ex_scope),
            oauth_authorization_codes::expires_at.eq(expires_at),
            oauth_authorization_codes::created_at.eq(now_ms),
        ))
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to store auth code: {}", e)))?;

    tracing::info!(
        "OAuth2 provider_callback: generated auth code for user '{}', redirecting to Element X",
        matrix_user_id
    );

    // Build redirect URL back to Element X
    // Custom schemes like io.element:/callback can't be parsed by url::Url,
    // so we build the redirect string manually
    let separator = if session.ex_redirect_uri.contains('?') { "&" } else { "?" };
    let redirect = format!(
        "{}{}code={}&state={}",
        session.ex_redirect_uri,
        separator,
        percent_encode(&palpo_code),
        percent_encode(&session.ex_state),
    );

    res.render(Redirect::found(redirect));
    Ok(())
}

/// POST /oauth2/token
///
/// Token endpoint. Handles both authorization_code and refresh_token grants.
#[endpoint]
async fn token_exchange(req: &mut Request) -> JsonResult<TokenResponse> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    if !oidc.enable_auth_server {
        return Err(MatrixError::not_found("Authorization server not enabled").into());
    }

    // Parse form-encoded body
    let grant_type = req.form::<String>("grant_type").await
        .ok_or_else(|| MatrixError::invalid_param("Missing grant_type"))?;

    match grant_type.as_str() {
        "authorization_code" => handle_authorization_code_grant(req, conf).await,
        "refresh_token" => handle_refresh_token_grant(req, conf).await,
        _ => Err(MatrixError::invalid_param(format!("Unsupported grant_type: {}", grant_type)).into()),
    }
}

/// POST /oauth2/revoke
///
/// Token revocation (RFC 7009). Always returns 200 OK.
#[endpoint]
async fn token_revoke(req: &mut Request, res: &mut Response) {
    let token = req.form::<String>("token").await;

    if let Some(token) = token {
        // Try to find and remove the access token
        let result = diesel::delete(
            user_access_tokens::table.filter(user_access_tokens::token.eq(&token)),
        )
        .execute(&mut connect().unwrap());

        if let Ok(deleted) = result {
            if deleted > 0 {
                tracing::info!("OAuth2 revoke: revoked access token");
            }
        }

        // Also try refresh tokens
        let _ = diesel::delete(
            user_refresh_tokens::table.filter(user_refresh_tokens::token.eq(&token)),
        )
        .execute(&mut connect().unwrap());
    }

    // RFC 7009: always return 200
    res.status_code(StatusCode::OK);
}

// =================== GRANT HANDLERS ===================

async fn handle_authorization_code_grant(
    req: &mut Request,
    conf: &'static config::ServerConfig,
) -> JsonResult<TokenResponse> {
    let code = req.form::<String>("code").await
        .ok_or_else(|| MatrixError::invalid_param("Missing code"))?;
    let client_id = req.form::<String>("client_id").await
        .ok_or_else(|| MatrixError::invalid_param("Missing client_id"))?;
    let redirect_uri = req.form::<String>("redirect_uri").await
        .ok_or_else(|| MatrixError::invalid_param("Missing redirect_uri"))?;
    let code_verifier = req.form::<String>("code_verifier").await
        .ok_or_else(|| MatrixError::invalid_param("Missing code_verifier"))?;

    // Look up authorization code
    let auth_code = oauth_authorization_codes::table
        .filter(oauth_authorization_codes::code.eq(&code))
        .first::<(String, String, String, String, String, String, String, i64, i64)>(&mut connect()?)
        .optional()
        .map_err(|e| MatrixError::unknown(format!("DB error: {}", e)))?;

    let auth_code = auth_code.ok_or_else(|| MatrixError::invalid_param("Invalid or expired authorization code"))?;

    let (stored_code, stored_client_id, stored_user_id, stored_redirect_uri, stored_challenge, _challenge_method, stored_scope, expires_at, _created_at) = auth_code;

    // Verify not expired
    if (UnixMillis::now().get() as i64) > expires_at {
        // Delete expired code
        let _ = diesel::delete(oauth_authorization_codes::table.filter(oauth_authorization_codes::code.eq(&stored_code)))
            .execute(&mut connect()?);
        return Err(MatrixError::invalid_param("Authorization code has expired").into());
    }

    // Verify client_id matches
    if client_id != stored_client_id {
        return Err(MatrixError::invalid_param("client_id mismatch").into());
    }

    // Verify redirect_uri matches
    if redirect_uri != stored_redirect_uri {
        return Err(MatrixError::invalid_param("redirect_uri mismatch").into());
    }

    // Verify PKCE: BASE64URL(SHA256(code_verifier)) == stored_code_challenge
    let mut hasher = sha2::Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let computed_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    if computed_challenge != stored_challenge {
        return Err(MatrixError::invalid_param("PKCE verification failed").into());
    }

    // Delete used code (one-time use)
    diesel::delete(oauth_authorization_codes::table.filter(oauth_authorization_codes::code.eq(&stored_code)))
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to delete auth code: {}", e)))?;

    // Create Matrix device and tokens for the user
    let user_id = crate::core::identifiers::UserId::parse(&stored_user_id)
        .map_err(|_| MatrixError::unknown("Invalid stored user_id"))?;

    let user = users::table
        .filter(users::id.eq(&user_id))
        .first::<DbUser>(&mut connect()?)
        .map_err(|_| MatrixError::not_found("User not found"))?;

    let device_id: OwnedDeviceId = format!("OIDC_{}", utils::random_string(8)).into();
    let access_token = utils::random_string(64);
    let refresh_token_str = utils::random_string(64);

    // Create device
    let new_device = crate::data::user::NewDbUserDevice {
        user_id: user.id.clone(),
        device_id: device_id.clone(),
        display_name: Some("Element X (OIDC)".to_string()),
        user_agent: Some("Element X".to_string()),
        is_hidden: false,
        last_seen_ip: None,
        last_seen_at: Some(UnixMillis::now()),
        created_at: UnixMillis::now(),
    };

    diesel::insert_into(user_devices::table)
        .values(&new_device)
        .on_conflict((user_devices::user_id, user_devices::device_id))
        .do_update()
        .set(user_devices::last_seen_at.eq(Some(UnixMillis::now())))
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to create device: {}", e)))?;

    // Create refresh token
    let expires_at = UnixMillis::now().get() + conf.refresh_token_ttl;
    let ultimate_expires = UnixMillis::now().get() + conf.session_ttl;
    let refresh_token_id = data::user::device::set_refresh_token(
        &user.id,
        &device_id,
        &refresh_token_str,
        expires_at,
        ultimate_expires,
    )?;

    // Create access token with oauth_client_id binding
    let new_token = crate::data::user::NewDbAccessToken {
        user_id: user.id.clone(),
        device_id: device_id.clone(),
        token: access_token.clone(),
        puppets_user_id: None,
        last_validated: Some(UnixMillis::now()),
        refresh_token_id: Some(refresh_token_id),
        is_used: false,
        expires_at: Some(UnixMillis(UnixMillis::now().get() + 300_000)), // 5 min
        created_at: UnixMillis::now(),
        oauth_client_id: Some(client_id),
    };

    diesel::insert_into(user_access_tokens::table)
        .values(&new_token)
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to create access token: {}", e)))?;

    tracing::info!("OAuth2 token: issued tokens for user '{}' device '{}'", user.id, device_id);

    json_ok(TokenResponse {
        access_token,
        refresh_token: Some(refresh_token_str),
        token_type: "Bearer".into(),
        expires_in: Some(300),
        scope: stored_scope,
    })
}

async fn handle_refresh_token_grant(
    req: &mut Request,
    conf: &'static config::ServerConfig,
) -> JsonResult<TokenResponse> {
    let refresh_token = req.form::<String>("refresh_token").await
        .ok_or_else(|| MatrixError::invalid_param("Missing refresh_token"))?;
    let client_id = req.form::<String>("client_id").await
        .ok_or_else(|| MatrixError::invalid_param("Missing client_id"))?;

    // Find existing refresh token
    let existing = user_refresh_tokens::table
        .filter(user_refresh_tokens::token.eq(&refresh_token))
        .first::<crate::data::user::DbRefreshToken>(&mut connect()?)
        .optional()
        .map_err(|e| MatrixError::unknown(format!("DB error: {}", e)))?;

    let existing = existing.ok_or_else(|| MatrixError::invalid_param("Invalid refresh_token"))?;

    // Verify the token's associated access token has the same oauth_client_id
    let access_token_entry = user_access_tokens::table
        .filter(user_access_tokens::refresh_token_id.eq(Some(existing.id)))
        .first::<crate::data::user::DbAccessToken>(&mut connect()?)
        .optional()
        .map_err(|e| MatrixError::unknown(format!("DB error: {}", e)))?;

    if let Some(ref at) = access_token_entry {
        if at.oauth_client_id.as_deref() != Some(&client_id) {
            return Err(MatrixError::invalid_param("client_id does not match token binding").into());
        }
    }

    // Generate new tokens
    let new_access_token = utils::random_string(64);
    let new_refresh_token = utils::random_string(64);
    let expires_at = UnixMillis::now().get() + conf.refresh_token_ttl;
    let ultimate_expires = UnixMillis::now().get() + conf.session_ttl;

    let new_refresh_id = data::user::device::set_refresh_token(
        &existing.user_id,
        &existing.device_id,
        &new_refresh_token,
        expires_at,
        ultimate_expires,
    )?;

    // Update access token
    if let Some(at) = access_token_entry {
        data::user::device::set_access_token(
            &at.user_id,
            &at.device_id,
            &new_access_token,
            Some(new_refresh_id),
        )?;
    }

    json_ok(TokenResponse {
        access_token: new_access_token,
        refresh_token: Some(new_refresh_token),
        token_type: "Bearer".into(),
        expires_in: Some(300),
        scope: "urn:matrix:org.matrix.msc2967.client:api:*".into(),
    })
}

// =================== HELPERS ===================

/// Simple percent-encoding for URL query values
fn percent_encode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn generate_pkce() -> (String, String) {
    let verifier = utils::random_string(96);
    let mut hasher = sha2::Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
    (verifier, challenge)
}

/// Exchange provider authorization code for tokens.
/// Similar to oidc.rs exchange_code_for_tokens but uses our provider_callback URL.
async fn exchange_provider_code(
    code: &str,
    provider_config: &OidcProviderConfig,
    provider_info: &super::client::oidc::OidcProviderInfo,
    callback_url: &str,
    code_verifier: Option<&str>,
) -> Result<super::client::oidc::OAuthTokenResponse, MatrixError> {
    let client = reqwest::Client::new();

    let mut params = vec![
        ("client_id", provider_config.client_id.as_str()),
        ("client_secret", provider_config.client_secret.as_str()),
        ("code", code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", callback_url),
    ];

    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier));
    }

    let provider_type = super::client::oidc::ProviderType::from_issuer(&provider_config.issuer);
    let request = match provider_type {
        super::client::oidc::ProviderType::GitHub => client
            .post(&provider_info.token_endpoint)
            .header("Accept", "application/json"),
        _ => client.post(&provider_info.token_endpoint),
    };

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|e| MatrixError::unknown(format!("Token exchange failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(MatrixError::unknown(format!("Token exchange failed: HTTP {} - {}", status, text)));
    }

    response
        .json()
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to parse token response: {}", e)))
}
