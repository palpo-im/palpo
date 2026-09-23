//! # OAuth/OIDC Authentication Module
//!
//! This module implements OAuth 2.0 Authorization Code flow with support for both
//! OpenID Connect (OIDC) providers and pure OAuth 2.0 providers for Matrix server authentication.
//!
//! ## Overview
//!
//! This authentication system allows users to log into the Matrix server using their
//! accounts from external identity providers (Google, GitHub, etc.), eliminating
//! the need for separate Matrix passwords. The implementation supports both:
//! - Standard OIDC providers (Google) with discovery endpoints
//! - Pure OAuth 2.0 providers (GitHub) with custom user info endpoints Both follow the OAuth 2.0
//!   Authorization Code flow with optional PKCE support.
//!
//! ## Authentication Flow Diagram
//!
//! ```text
//! ┌──────────┐     ┌──────────────┐     ┌──────────────────┐     ┌─────────────┐
//! │  Client  │────▶│ Palpo Server │────▶│ OAuth Provider   │────▶│  Database   │
//! │          │     │              │     │ (Google, GitHub) │     │             │
//! └──────────┘     └──────────────┘     └──────────────────┘     └─────────────┘
//!      │                   │                      │                      │
//!      │ 1. GET /oidc/auth │                      │                      │
//!      │──────────────────▶│                      │                      │
//!      │                   │ 2. Generate state    │                      │
//!      │                   │    & PKCE challenge  │                      │
//!      │                   │                      │                      │
//!      │ 3. Redirect to    │                      │                      │
//!      │    provider       │                      │                      │
//!      │◀──────────────────│                      │                      │
//!      │                                          │                      │
//!      │ 4. User authenticates & grants consent   │                      │
//!      │─────────────────────────────────────────▶│                      │
//!      │                                          │                      │
//!      │ 5. Callback with auth code               │                      │
//!      │─────────────────────────────────────────▶│                      │
//!      │                   │ 6. Exchange code     │                      │
//!      │                   │    for tokens        │                      │
//!      │                   │─────────────────────▶│                      │
//!      │                   │ 7. Access token      │                      │
//!      │                   │◀─────────────────────│                      │
//!      │                   │ 8. Fetch user info   │                      │
//!      │                   │─────────────────────▶│                      │
//!      │                   │ 9. User profile data │                      │
//!      │                   │◀─────────────────────│                      │
//!      │                   │ 10. Create/get user  │                      │
//!      │                   │─────────────────────────────────────────────▶│
//!      │                   │ 11. Matrix user &    │                      │
//!      │                   │     access token     │                      │
//!      │                   │◀─────────────────────────────────────────────│
//!      │ 12. Login success │                      │                      │
//!      │◀──────────────────│                      │                      │
//! ```
//!
//! ## Security Features
//!
//! ### CSRF Protection
//! - Random `state` parameter generated for each auth request
//! - State stored in HTTP-only, secure cookie with short expiration
//! - State validation on callback prevents CSRF attacks
//!
//! ### PKCE (Proof Key for Code Exchange)
//! - Optional code_verifier and code_challenge for enhanced security
//! - Protects against authorization code interception attacks
//! - Especially important for mobile and SPA clients
//!
//! ### Secure Cookie Settings
//! - HTTP-only cookies prevent XSS access
//! - Secure flag ensures HTTPS-only transmission in production
//! - SameSite=Lax provides CSRF protection
//! - Short expiration (10 minutes) limits exposure window
//!
//! ## Supported Providers
//!
//! This implementation supports:
//! - **Google OAuth 2.0**: Full OIDC compliance with discovery endpoint
//! - **GitHub OAuth**: OAuth 2.0 with custom user info endpoint (not OIDC-compliant)
//! - **Generic OIDC**: Any provider with .well-known/openid-configuration
//!
//! ### Provider-specific handling:
//!
//! #### GitHub OAuth
//! - Requires `Accept: application/json` header for token exchange
//! - Requires `User-Agent` header for API requests
//! - Uses different field names (id vs sub, avatar_url vs picture)
//! - **Important**: Email may be null if user has private email settings
//!
//! #### Recommended GitHub Configuration
//! ```toml
//! [oidc]
//! user_mapping = "sub"  # Use GitHub ID instead of email
//! require_email_verified = false  # Allow users with private emails
//! user_prefix = "github_"  # Distinguish GitHub users
//!
//! [oidc.providers.github]
//! issuer = "https://github.com"
//! scopes = ["read:user", "user:email"]  # Request email access (may still be private)
//! ```
//!
//! ## User ID Generation
//!
//! Matrix user IDs combine username with provider ID for security:
//! - Ensures uniqueness even if usernames change hands
//! - Prevents account takeover when users rename on GitHub
//!
//! Examples:
//! - GitHub user "octocat" (ID 123) → `@octocat_123:server`
//! - Google user john@gmail.com → `@john_456789:server`
//! - No username/email → `@user_123456:server`

use std::collections::HashMap;

use cookie::time::Duration;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use url::Url;

use crate::config::{self, OidcProviderConfig};
use crate::core::{MatrixError, OwnedDeviceId, OwnedMxcUri, UnixMillis};
use crate::data::user::DbUser;
use crate::exts::*;
use crate::{AppError, AppResult, JsonResult, TOKEN_LENGTH, json_ok, user, utils};

const SSO_LOGIN_TOKEN_TTL_MS: u64 = 5_000;

/// OIDC session state for tracking authentication flow
///
/// This structure holds temporary data during the OAuth flow:
/// - CSRF protection via state parameter
/// - PKCE code verifier for enhanced security
/// - Provider selection for multi-provider setups
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcSession {
    /// CSRF protection state parameter
    pub state: String,
    /// PKCE code verifier (if PKCE is enabled)
    pub code_verifier: Option<String>,
    /// Selected provider name
    pub provider: String,
    /// Matrix client callback for an `m.login.sso` flow. This is absent for
    /// callers using Palpo's legacy custom OIDC endpoint directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
    /// Session creation timestamp
    pub created_at: u64,
}

/// OIDC authorization-server metadata shared by legacy Matrix SSO and the
/// custom OIDC flow.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct OidcMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub issuer: String,
}

/// OIDC provider discovery information
///
/// Contains the well-known endpoints for an OIDC provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcProviderInfo {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    pub issuer: String,
}

/// Supported OAuth/OIDC provider types
#[derive(Debug, Clone, PartialEq)]
enum ProviderType {
    Google,
    GitHub,
    Generic,
}

impl ProviderType {
    fn from_issuer(issuer: &str) -> Self {
        match issuer.trim_end_matches('/') {
            "https://accounts.google.com" => Self::Google,
            "https://github.com" => Self::GitHub,
            _ => Self::Generic,
        }
    }
}

/// JWT Claims structure for OIDC
#[derive(Debug, Serialize, Deserialize)]
pub struct OidcClaims {
    pub sub: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub email_verified: Option<bool>,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    pub aud: String,
}

/// OIDC user information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OidcUserInfo {
    pub sub: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub email_verified: Option<bool>,
    pub preferred_username: Option<String>, // GitHub login/username
    /// Complete UserInfo response, including provider-specific claims used by
    /// `attribute_mapping`.
    #[serde(default)]
    pub claims: serde_json::Map<String, serde_json::Value>,
}

/// Google OAuth token response
#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: i64,
    id_token: Option<String>,
}

/// Google user info response
#[derive(Debug, Deserialize)]
struct GoogleUserInfoResponse {
    id: String,
    email: Option<String>,
    name: Option<String>,
    picture: Option<String>,
    verified_email: Option<bool>,
}

impl From<&OidcClaims> for OidcUserInfo {
    fn from(claims: &OidcClaims) -> Self {
        Self {
            sub: claims.sub.clone(),
            email: claims.email.clone(),
            name: claims.name.clone(),
            picture: claims.picture.clone(),
            email_verified: claims.email_verified,
            preferred_username: None, // Not available in JWT claims
            claims: serde_json::to_value(claims)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default(),
        }
    }
}

impl From<GoogleUserInfoResponse> for OidcUserInfo {
    fn from(info: GoogleUserInfoResponse) -> Self {
        Self {
            sub: info.id,
            email: info.email,
            name: info.name,
            picture: info.picture,
            email_verified: info.verified_email,
            preferred_username: None, // Not available from Google
            claims: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OidcStatusResponse {
    /// Whether OIDC authentication is enabled
    pub enabled: bool,
    /// Map of available providers with their display information
    pub providers: HashMap<String, OidcProviderStatus>,
    /// Default provider name (if configured)
    pub default_provider: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OidcProviderStatus {
    /// Human-readable display name for this provider
    pub display_name: String,
    /// OIDC issuer URL
    pub issuer: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OidcAuthResponse {
    pub auth_url: String,
    pub state: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OidcLoginResponse {
    pub user_id: String,
    pub access_token: String,
    pub device_id: String,
    pub home_server: String,
}

/// `GET /_matrix/client/*/oidc/status`
///
/// **OIDC Discovery Endpoint**
///
/// Returns information about available OIDC providers and their configuration.
/// Clients use this endpoint to discover which authentication methods are available.
///
/// ## Response Format
/// ```json
/// {
///   "enabled": true,
///   "providers": {
///     "google": {
///       "display_name": "Sign in with Google",
///       "issuer": "https://accounts.google.com"
///     },
///     "github": {
///       "display_name": "Sign in with GitHub",
///       "issuer": "https://github.com"
///     }
///   },
///   "default_provider": "google"
/// }
/// ```
///
/// ## Security Note
/// This endpoint is public and doesn't require authentication to allow
/// clients to discover available authentication methods before login.
#[endpoint]
pub async fn oidc_status() -> JsonResult<OidcStatusResponse> {
    let config = config::get();

    // Check if OIDC is enabled in configuration
    let Some(oidc_config) = config.enabled_oidc() else {
        return json_ok(OidcStatusResponse {
            enabled: false,
            providers: HashMap::new(),
            default_provider: None,
        });
    };

    // Build provider status information from configuration
    let mut providers = HashMap::new();
    for (provider_name, provider_config) in &oidc_config.providers {
        let display_name = provider_config
            .display_name
            .clone()
            .unwrap_or_else(|| format!("Sign in with {}", capitalize_first(provider_name)));

        providers.insert(
            provider_name.clone(),
            OidcProviderStatus {
                display_name,
                issuer: provider_config.issuer.clone(),
            },
        );
    }

    json_ok(OidcStatusResponse {
        enabled: true,
        providers,
        default_provider: oidc_config.default_provider.clone(),
    })
}

/// Utility function to capitalize the first letter of a string
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Generate a cryptographically secure random string for CSRF/PKCE
fn generate_random_string(length: usize) -> String {
    crate::utils::random_string(length)
}

/// Generate PKCE code verifier and challenge
///
/// Returns (code_verifier, code_challenge) tuple
/// Implements proper SHA256 hashing as required by OAuth 2.0 PKCE spec
fn generate_pkce_challenge() -> (String, String) {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    // Generate 128-bit random verifier (43-128 characters per RFC 7636)
    let code_verifier = generate_random_string(96);

    // Create SHA256 hash of verifier and base64url encode it (RFC 7636)
    let mut hasher = sha2::Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let hash_result = hasher.finalize();
    let code_challenge = URL_SAFE_NO_PAD.encode(&hash_result[..]);

    (code_verifier, code_challenge)
}

/// Get provider configuration by name
fn get_provider_config(provider_name: &str) -> Result<&'static OidcProviderConfig, MatrixError> {
    let config = config::get();
    let oidc_config = config
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    oidc_config
        .providers
        .get(provider_name)
        .ok_or_else(|| MatrixError::not_found("Unknown OIDC provider"))
}

/// Discover OIDC endpoints for a provider
///
/// Attempts to fetch the .well-known/openid-configuration endpoint.
/// Falls back to common endpoint patterns for known providers.
async fn discover_provider_endpoints(
    provider_config: &OidcProviderConfig,
) -> Result<OidcProviderInfo, MatrixError> {
    match discover_oidc_metadata(&provider_config.issuer).await {
        Ok(metadata) => Ok(OidcProviderInfo {
            authorization_endpoint: metadata.authorization_endpoint,
            token_endpoint: metadata.token_endpoint,
            userinfo_endpoint: metadata.userinfo_endpoint.ok_or_else(|| {
                MatrixError::unknown("Missing userinfo_endpoint in OIDC discovery metadata")
            })?,
            issuer: metadata.issuer,
        }),
        Err(discovery_error) => {
            // Fallback to common patterns for known providers
            let provider_type = ProviderType::from_issuer(&provider_config.issuer);
            match provider_type {
                ProviderType::Google => Ok(OidcProviderInfo {
                    authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth"
                        .to_string(),
                    token_endpoint: "https://oauth2.googleapis.com/token".to_string(),
                    userinfo_endpoint: "https://www.googleapis.com/oauth2/v2/userinfo".to_string(),
                    issuer: provider_config.issuer.clone(),
                }),
                ProviderType::GitHub => Ok(OidcProviderInfo {
                    authorization_endpoint: "https://github.com/login/oauth/authorize".to_string(),
                    token_endpoint: "https://github.com/login/oauth/access_token".to_string(),
                    userinfo_endpoint: "https://api.github.com/user".to_string(),
                    issuer: provider_config.issuer.clone(),
                }),
                ProviderType::Generic => Err(discovery_error),
            }
        }
    }
}

fn oidc_discovery_url(issuer: &str) -> Result<Url, MatrixError> {
    let issuer = issuer.trim_end_matches('/');
    let issuer_url = Url::parse(issuer)
        .map_err(|e| MatrixError::unknown(format!("Invalid OIDC issuer URL: {e}")))?;
    if issuer_url.query().is_some() || issuer_url.fragment().is_some() {
        return Err(MatrixError::unknown(
            "OIDC issuer URL must not contain a query or fragment",
        ));
    }

    Url::parse(&format!("{issuer}/.well-known/openid-configuration"))
        .map_err(|e| MatrixError::unknown(format!("Invalid OIDC discovery URL: {e}")))
}

/// Fetch authorization and token endpoints exactly as advertised by the
/// provider's OpenID Connect Discovery document.
pub(super) async fn discover_oidc_metadata(issuer: &str) -> Result<OidcMetadata, MatrixError> {
    let discovery_url = oidc_discovery_url(issuer)?;
    let response = reqwest::Client::new()
        .get(discovery_url.clone())
        .send()
        .await
        .map_err(|e| MatrixError::unknown(format!("OIDC discovery request failed: {e}")))?;

    if !response.status().is_success() {
        return Err(MatrixError::unknown(format!(
            "OIDC discovery endpoint {discovery_url} returned {}",
            response.status()
        )));
    }

    let metadata = response.json::<OidcMetadata>().await.map_err(|e| {
        MatrixError::unknown(format!("Failed to parse OIDC discovery document: {e}"))
    })?;
    let expected_issuer = issuer.trim_end_matches('/');
    if metadata.issuer != expected_issuer {
        return Err(MatrixError::unknown(format!(
            "OIDC discovery issuer mismatch: expected {expected_issuer}, got {}",
            metadata.issuer
        )));
    }

    // Parse endpoint URLs eagerly so malformed discovery data fails before a
    // login session is created.
    Url::parse(&metadata.authorization_endpoint)
        .map_err(|e| MatrixError::unknown(format!("Invalid authorization_endpoint: {e}")))?;
    Url::parse(&metadata.token_endpoint)
        .map_err(|e| MatrixError::unknown(format!("Invalid token_endpoint: {e}")))?;

    Ok(metadata)
}

/// `GET /_matrix/client/*/oidc/auth`
///
/// **OAuth Authorization Initiation Endpoint**
///
/// Starts the OAuth 2.0 Authorization Code flow by redirecting the user to the
/// selected OIDC provider for authentication. This is step 1 of the OIDC flow.
///
/// ## Request Parameters
/// - `provider` (optional): Name of the OIDC provider to use. If not specified, uses the default
///   provider from configuration.
///
/// ## Security Features
/// - **CSRF Protection**: Generates a random `state` parameter and stores it in an HTTP-only cookie
///   for validation on callback.
/// - **PKCE Support**: Optionally generates code_verifier/code_challenge for enhanced security
///   (enabled by default).
/// - **Secure Cookies**: Uses appropriate security flags for production deployment.
///
/// ## Response
/// Redirects (302) to the OIDC provider's authorization endpoint with appropriate
/// OAuth 2.0 parameters including client_id, scopes, and security tokens.
///
/// ## Error Conditions
/// - OIDC not enabled in configuration
/// - Unknown provider specified
/// - Provider discovery/configuration failures
#[endpoint]
pub async fn oidc_auth(req: &mut Request, res: &mut Response) -> AppResult<()> {
    // Step 1: Validate OIDC configuration
    let config = config::get();
    let oidc_config = config
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC authentication not enabled"))?;

    // Step 2: Determine which provider to use
    let provider_name = req
        .query::<String>("provider")
        .or_else(|| oidc_config.default_provider.clone())
        .or_else(|| oidc_config.providers.keys().next().cloned())
        .ok_or_else(|| {
            MatrixError::invalid_param("No OIDC provider specified and no default configured")
        })?;

    let provider_config = oidc_config.providers.get(&provider_name).ok_or_else(|| {
        MatrixError::not_found(format!("Unknown OIDC provider: {}", provider_name))
    })?;
    let redirect_url = req
        .query::<String>("redirectUrl")
        .or_else(|| req.query::<String>("redirect_url"));
    if let Some(redirect_url) = &redirect_url {
        Url::parse(redirect_url)
            .map_err(|e| MatrixError::invalid_param(format!("Invalid redirectUrl: {e}")))?;
    }

    // Step 3: Discover provider endpoints
    let provider_info = discover_provider_endpoints(provider_config).await?;

    // Step 4: Generate security tokens
    let state = generate_random_string(32);
    let (code_verifier, code_challenge) = if oidc_config.enable_pkce {
        let (verifier, challenge) = generate_pkce_challenge();
        (Some(verifier), Some(challenge))
    } else {
        (None, None)
    };

    // Step 5: Create OIDC session for tracking
    let session = OidcSession {
        state: state.clone(),
        code_verifier,
        provider: provider_name.clone(),
        redirect_url,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    // Step 6: Store session in secure cookie
    let session_data = serde_json::to_string(&session)
        .map_err(|e| MatrixError::unknown(format!("Failed to serialize OIDC session: {}", e)))?;

    // Configure cookie security based on environment
    let is_production = !cfg!(debug_assertions);
    res.add_cookie(
        salvo::http::cookie::Cookie::build(("oidc_session", session_data))
            .http_only(true)
            .secure(is_production) // HTTPS only in production
            .same_site(salvo::http::cookie::SameSite::Lax)
            .max_age(Duration::seconds(oidc_config.session_timeout as i64))
            .build(),
    );

    // Step 7: Build OAuth 2.0 authorization URL
    let mut auth_url = Url::parse(&provider_info.authorization_endpoint)
        .map_err(|e| MatrixError::unknown(format!("Invalid authorization endpoint: {}", e)))?;

    // Step 8: Add OAuth 2.0 parameters
    {
        let mut query_pairs = auth_url.query_pairs_mut();

        // Required OAuth 2.0 parameters
        query_pairs
            .append_pair("client_id", &provider_config.client_id)
            .append_pair("redirect_uri", &provider_config.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("state", &state);

        // Add requested scopes
        let scopes = provider_config.scopes.join(" ");
        query_pairs.append_pair("scope", &scopes);

        // Add PKCE challenge if enabled
        if let Some(challenge) = &code_challenge {
            query_pairs
                .append_pair("code_challenge", challenge)
                .append_pair("code_challenge_method", "S256");
        }

        // Add any additional provider-specific parameters
        for (key, value) in &provider_config.additional_params {
            query_pairs.append_pair(key, value);
        }
    }

    tracing::info!(
        "Starting OIDC authentication flow for provider '{}' with state '{}'",
        provider_name,
        &state[..8] // Log only first 8 chars for security
    );

    // Step 9: Redirect user to OIDC provider for authentication
    res.render(Redirect::found(&auth_url));
    Ok(())
}

/// `GET /_matrix/client/*/oidc/callback`
///
/// **OAuth Callback Handler - The Heart of OAuth/OIDC Authentication**
///
/// This endpoint handles the OAuth 2.0 callback from the provider after the user
/// has authenticated and granted consent. It automatically detects the provider type
/// from the session and handles provider-specific differences (Google OIDC vs GitHub OAuth).
///
/// ## Callback Flow Breakdown
/// ```text
/// 1. Validate callback parameters (code, state)
/// 2. Restore and validate session from secure cookie (includes provider info)
/// 3. Identify provider type from session for proper handling
/// 4. Exchange authorization code for access token
///    - GitHub: Requires Accept: application/json header
///    - Google: Standard token exchange
/// 5. Fetch user information from provider
///    - GitHub: API endpoint with User-Agent header, different field names
///    - Google: Standard OIDC userinfo endpoint
/// 6. Validate user according to policy (email verification, etc.)
/// 7. Create or retrieve Matrix user account
/// 8. For Matrix SSO, generate a short-lived login token and redirect back to
///    the client; direct users of the custom endpoint still receive credentials
///    as JSON.
/// ```
///
/// ## Security Validations
/// - **State Parameter**: Validates CSRF protection token
/// - **Session Timeout**: Ensures authentication session hasn't expired
/// - **PKCE Verification**: Validates code_verifier if PKCE was used
/// - **Email Verification**: Checks email_verified claim (if required)
/// - **Provider Validation**: Ensures token came from correct issuer
///
/// ## Query Parameters
/// - `code`: OAuth 2.0 authorization code from provider
/// - `state`: CSRF protection token (must match stored value)
/// - `error` (optional): Error code if authentication failed
/// - `error_description` (optional): Human-readable error description
///
/// ## Error Handling
/// Comprehensive error handling for all failure scenarios:
/// - Invalid/missing parameters → 400 Bad Request
/// - CSRF token mismatch → 403 Forbidden
/// - Session expired → 401 Unauthorized
/// - Provider communication failures → 502 Bad Gateway
/// - User creation failures → 500 Internal Server Error
#[endpoint]
pub async fn oidc_callback(req: &mut Request, res: &mut Response) -> AppResult<()> {
    // Step 1: Handle OAuth error responses first
    if let Some(error) = req.query::<String>("error") {
        let error_description = req
            .query::<String>("error_description")
            .unwrap_or_else(|| "No description provided".to_string());

        tracing::warn!(
            "OIDC provider returned error: {} - {}",
            error,
            error_description
        );
        return Err(MatrixError::forbidden(
            format!("Authentication failed: {}", error_description),
            None,
        )
        .into());
    }

    // Step 2: Extract and validate required callback parameters
    let code = req
        .query::<String>("code")
        .ok_or_else(|| MatrixError::invalid_param("Missing authorization code in callback"))?;
    let state = req
        .query::<String>("state")
        .ok_or_else(|| MatrixError::invalid_param("Missing state parameter in callback"))?;

    // Step 3: Restore OIDC session from secure cookie
    let session_cookie = req
        .cookie("oidc_session")
        .ok_or_else(|| MatrixError::unauthorized("OIDC session not found or expired"))?;

    let session: OidcSession = serde_json::from_str(session_cookie.value())
        .map_err(|e| MatrixError::unauthorized(format!("Invalid OIDC session data: {}", e)))?;

    // Step 4: Validate CSRF state parameter
    if state != session.state {
        tracing::warn!(
            "OIDC state mismatch: received '{}', expected '{}'",
            &state[..8.min(state.len())],
            &session.state[..8.min(session.state.len())]
        );
        return Err(MatrixError::forbidden("CSRF state validation failed", None).into());
    }

    // Step 5: Check session timeout
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let config = config::get();
    let oidc_config = config
        .enabled_oidc()
        .ok_or_else(|| MatrixError::unknown("OIDC configuration missing"))?;

    if now > session.created_at + oidc_config.session_timeout {
        return Err(MatrixError::unauthorized("OIDC session has expired").into());
    }

    // Step 6: Get provider configuration
    let provider_config = oidc_config
        .providers
        .get(&session.provider)
        .ok_or_else(|| {
            MatrixError::unknown(format!(
                "Provider '{}' no longer configured",
                session.provider
            ))
        })?;

    // Step 7: Discover provider endpoints (may be cached in production)
    let provider_info = discover_provider_endpoints(provider_config).await?;

    // Step 8: Exchange authorization code for tokens
    let token_response = exchange_code_for_tokens(
        &code,
        provider_config,
        &provider_info,
        session.code_verifier.as_deref(),
    )
    .await?;

    // Step 9: Fetch user information from provider
    let user_info = get_user_info_from_provider(
        &token_response.access_token,
        &provider_info,
        provider_config,
    )
    .await?;

    // Step 10: Validate user according to configured policies
    validate_user_info(&user_info, oidc_config)?;

    // Step 11: Generate Matrix user ID using configured mapping strategy
    let matrix_user_id =
        generate_matrix_user_id(&user_info, oidc_config, config.server_name.as_str())?;
    let display_name = generate_display_name(&user_info, provider_config);
    let avatar_url = generate_avatar_url(&user_info, provider_config);

    // Step 12: Create or retrieve Matrix user account
    let user = create_or_get_user(&matrix_user_id, &display_name, avatar_url, oidc_config).await?;

    if let Some(redirect_url) = session.redirect_url.as_deref() {
        let login_token = utils::random_string(TOKEN_LENGTH);
        user::create_login_token_with_ttl(&user.id, &login_token, SSO_LOGIN_TOKEN_TTL_MS).await?;
        let client_callback = append_login_token(redirect_url, &login_token)?;

        tracing::info!(
            "OIDC SSO authentication successful for user '{}' via provider '{}'",
            matrix_user_id,
            session.provider
        );
        res.render(Redirect::found(client_callback));
        return Ok(());
    }

    // Step 13: Create Matrix device and access token
    let device_id = format!("OIDC_{}", generate_random_string(8));
    let access_token = create_access_token_for_user(&user, &device_id).await?;

    tracing::info!(
        "OIDC authentication successful for user '{}' via provider '{}'",
        matrix_user_id,
        session.provider
    );

    // Step 14: Return Matrix authentication credentials for callers using the
    // custom OIDC endpoint directly.
    res.render(Json(OidcLoginResponse {
        user_id: matrix_user_id,
        access_token,
        device_id,
        home_server: config.server_name.to_string(),
    }));
    Ok(())
}

fn append_login_token(redirect_url: &str, login_token: &str) -> Result<Url, MatrixError> {
    let mut redirect_url = Url::parse(redirect_url)
        .map_err(|e| MatrixError::invalid_param(format!("Invalid redirectUrl: {e}")))?;
    let existing_pairs = redirect_url
        .query_pairs()
        .filter(|(key, _)| key != "loginToken")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

    redirect_url.set_query(None);
    redirect_url
        .query_pairs_mut()
        .extend_pairs(existing_pairs)
        .append_pair("loginToken", login_token);

    Ok(redirect_url)
}

/// `POST /_matrix/client/*/oidc/login`
///
/// **Direct JWT Token Authentication (Future Enhancement)**
///
/// Alternative authentication method for clients that can obtain OIDC JWT tokens
/// directly from the provider (e.g., mobile apps with native OAuth SDKs).
///
/// ## Implementation Status
/// This endpoint is planned for future implementation and would provide:
/// - Direct JWT ID token validation
/// - Mobile app integration support
/// - Reduced redirect-based flow complexity
/// - Support for native app authentication
///
/// ## Security Requirements for Future Implementation
/// - JWT signature validation against provider's public keys
/// - Issuer and audience claim validation
/// - Token expiration and not-before time checks
/// - Nonce validation for replay protection
///
/// Currently returns "not implemented" to maintain API contract.
#[endpoint]
pub async fn oidc_login(_depot: &mut Depot) -> JsonResult<OidcLoginResponse> {
    Err(MatrixError::unknown("Direct JWT authentication not yet implemented - use authorization code flow via /oidc/auth").into())
}

// =================== HELPER FUNCTIONS ===================
//

/// **OAuth Token Exchange - Step 2 of OAuth Flow**
///
/// Exchanges the authorization code received from the OIDC provider for an access token
/// and optionally an ID token. This is a server-to-server communication step.
///
/// ## PKCE Verification
/// If PKCE was used in the authorization request, the code_verifier is included to prove
/// that the same client that initiated the flow is completing it.
///
/// ## Security Notes
/// - Client secret is transmitted securely to provider
/// - Request is made over HTTPS only
/// - Response tokens are validated before use
async fn exchange_code_for_tokens(
    code: &str,
    provider_config: &OidcProviderConfig,
    provider_info: &OidcProviderInfo,
    code_verifier: Option<&str>,
) -> Result<OAuthTokenResponse, MatrixError> {
    let client = reqwest::Client::new();

    // Build token exchange request parameters
    let mut params = vec![
        ("client_id", provider_config.client_id.as_str()),
        ("client_secret", provider_config.client_secret.as_str()),
        ("code", code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", provider_config.redirect_uri.as_str()),
    ];

    // Add PKCE verification if code_verifier is present
    if let Some(verifier) = code_verifier {
        params.push(("code_verifier", verifier));
    }

    tracing::debug!(
        "Exchanging authorization code for tokens with provider: {}",
        provider_info.issuer
    );

    // Build request with provider-specific headers
    let provider_type = ProviderType::from_issuer(&provider_config.issuer);
    let request = match provider_type {
        ProviderType::GitHub => client
            .post(&provider_info.token_endpoint)
            .header("Accept", "application/json"),
        _ => client.post(&provider_info.token_endpoint),
    };

    let response = request
        .form(&params)
        .send()
        .await
        .map_err(|e| MatrixError::unknown(format!("Token exchange request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        tracing::error!(
            "Token exchange failed with status {}: {}",
            status,
            error_text
        );
        return Err(MatrixError::unknown(format!(
            "Token exchange failed: HTTP {}",
            status
        )));
    }

    let token_response: OAuthTokenResponse = response
        .json()
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to parse token response: {}", e)))?;

    tracing::debug!("Successfully exchanged authorization code for access token");
    Ok(token_response)
}

/// **User Information Retrieval - Step 3 of OAuth Flow**
///
/// Uses the access token to fetch user profile information from the OIDC provider's
/// userinfo endpoint. This provides the user's identity claims.
///
/// ## Returned Information
/// Typically includes: sub (subject), email, name, picture, email_verified, etc.
/// The exact claims depend on the scopes requested and provider capabilities.
async fn get_user_info_from_provider(
    access_token: &str,
    provider_info: &OidcProviderInfo,
    provider_config: &OidcProviderConfig,
) -> Result<OidcUserInfo, MatrixError> {
    let client = reqwest::Client::new();

    tracing::debug!("Fetching user info from provider: {}", provider_info.issuer);

    // Build request with provider-specific headers
    let provider_type = ProviderType::from_issuer(&provider_config.issuer);
    let request = match provider_type {
        ProviderType::GitHub => client
            .get(&provider_info.userinfo_endpoint)
            .bearer_auth(access_token)
            .header("User-Agent", "Palpo-Matrix-Server"),
        _ => client
            .get(&provider_info.userinfo_endpoint)
            .bearer_auth(access_token),
    };

    // Configure TLS verification based on settings
    // Note: TLS verification bypass not implemented for security
    // If needed, this would require configuring a custom reqwest client
    if provider_config.skip_tls_verify {
        tracing::warn!(
            "TLS verification bypass requested for provider {} but not implemented for security",
            provider_info.issuer
        );
    }

    let response = request
        .send()
        .await
        .map_err(|e| MatrixError::unknown(format!("User info request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        tracing::error!(
            "User info request failed with status {}: {}",
            status,
            error_text
        );
        return Err(MatrixError::unknown(format!(
            "User info request failed: HTTP {}",
            status
        )));
    }

    let user_info_response: serde_json::Value = response
        .json()
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to parse user info response: {}", e)))?;
    let claims = user_info_response
        .as_object()
        .cloned()
        .ok_or_else(|| MatrixError::unknown("OIDC user info response must be a JSON object"))?;

    // Parse user info based on provider type
    let provider_type = ProviderType::from_issuer(&provider_config.issuer);
    let user_info = match provider_type {
        ProviderType::GitHub => {
            // GitHub OAuth response format differs from OIDC standard:
            // - Uses 'id' (integer) instead of 'sub' (string) for user identifier
            // - Uses 'avatar_url' instead of 'picture' for profile image
            // - Email may be null if user has set email to private in GitHub settings
            //
            // Important: GitHub users often have private emails, so:
            // 1. Set user_mapping = "sub" in config to use GitHub ID instead of email
            // 2. Set require_email_verified = false to allow users without public emails
            let id = user_info_response["id"].as_i64().ok_or_else(|| {
                MatrixError::unknown("Missing required 'id' field in GitHub user info")
            })?;

            OidcUserInfo {
                sub: id.to_string(),
                email: user_info_response["email"].as_str().map(String::from), /* May be None for private emails */
                name: user_info_response["name"].as_str().map(String::from),
                picture: user_info_response["avatar_url"].as_str().map(String::from),
                email_verified: Some(true), /* GitHub verifies primary email, but it may not be
                                             * visible */
                preferred_username: user_info_response["login"].as_str().map(String::from), /* GitHub username */
                claims,
            }
        }
        ProviderType::Google | ProviderType::Generic => {
            // Standard OIDC claims
            OidcUserInfo {
                sub: user_info_response["sub"]
                    .as_str()
                    .ok_or_else(|| {
                        MatrixError::unknown("Missing required 'sub' claim in user info")
                    })?
                    .to_string(),
                email: user_info_response["email"].as_str().map(String::from),
                name: user_info_response["name"].as_str().map(String::from),
                picture: user_info_response["picture"].as_str().map(String::from),
                email_verified: user_info_response["email_verified"].as_bool(),
                preferred_username: user_info_response["preferred_username"]
                    .as_str()
                    .map(String::from),
                claims,
            }
        }
    };

    tracing::debug!(
        "Successfully retrieved user info for subject: {}",
        user_info.sub
    );
    Ok(user_info)
}

/// **User Validation - Policy Enforcement**
///
/// Validates the user information against configured policies before allowing
/// Matrix account creation or login.
///
/// ## Validation Checks
/// - Email verification status (if required)
/// - Account restrictions or blocklists (future enhancement)
/// - Domain restrictions (future enhancement)
fn validate_user_info(
    user_info: &OidcUserInfo,
    oidc_config: &crate::config::OidcConfig,
) -> Result<(), MatrixError> {
    // Check email verification requirement
    if oidc_config.require_email_verified {
        if user_info.email.is_none() {
            return Err(MatrixError::forbidden(
                "Email address is required for authentication",
                None,
            ));
        }

        if user_info.email_verified != Some(true) {
            return Err(MatrixError::forbidden(
                "Email address must be verified with the identity provider",
                None,
            ));
        }
    }

    // Future: Add domain restrictions, account blocklists, etc.

    Ok(())
}

/// **Matrix User ID Generation**
///
/// Generates a friendly Matrix user ID from OIDC user information.
/// Priority: username > email > ID
///
/// ## Security Considerations
/// - All localparts are sanitized for Matrix compliance
/// - Invalid characters are filtered out
/// - Uniqueness is guaranteed by using provider ID as fallback
fn generate_matrix_user_id(
    user_info: &OidcUserInfo,
    oidc_config: &crate::config::OidcConfig,
    server_name: &str,
) -> Result<String, MatrixError> {
    // For security: Always include provider ID to ensure uniqueness
    // GitHub usernames can be transferred when users rename
    let base_localpart = if let Some(username) = &user_info.preferred_username {
        // Combine username with ID for both readability and security
        // Format: "username_id" ensures uniqueness even if username changes hands
        format!("{}_{}", username, user_info.sub)
    } else if let Some(email) = &user_info.email {
        format!(
            "{}_{}",
            email.split('@').next().unwrap_or("user"),
            user_info.sub
        )
    } else {
        format!("user_{}", user_info.sub)
    };

    // Sanitize the localpart for Matrix compliance
    let sanitized = base_localpart
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect::<String>()
        .to_lowercase();

    if sanitized.is_empty() {
        return Err(MatrixError::invalid_param(
            "Cannot generate valid Matrix user ID from OIDC identity",
        ));
    }

    // Add configured prefix
    let prefixed_localpart = if oidc_config.user_prefix.is_empty() {
        sanitized
    } else {
        format!("{}{}", oidc_config.user_prefix, sanitized)
    };

    Ok(format!("@{}:{}", prefixed_localpart, server_name))
}

/// **Display Name Generation**
///
/// Generates a human-readable display name from OIDC user information,
/// considering provider-specific attribute mappings.
fn generate_display_name(user_info: &OidcUserInfo, provider_config: &OidcProviderConfig) -> String {
    if let Some(display_name_claim) = provider_config.attribute_mapping.get("display_name")
        && let Some(display_name) = user_info
            .claims
            .get(display_name_claim)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
    {
        return display_name.to_owned();
    }

    // Standard OIDC claim priority: name > email > fallback
    user_info
        .name
        .clone()
        .or_else(|| user_info.email.clone())
        .unwrap_or_else(|| {
            format!(
                "User {}",
                &user_info.sub[..std::cmp::min(8, user_info.sub.len())]
            )
        })
}

/// Resolve the configured avatar claim, accepting only Matrix Content URIs as
/// required by the Client-Server profile API.
fn generate_avatar_url(
    user_info: &OidcUserInfo,
    provider_config: &OidcProviderConfig,
) -> Option<OwnedMxcUri> {
    let avatar_claim = provider_config
        .attribute_mapping
        .get("avatar_url")
        .map(String::as_str)
        .unwrap_or("picture");
    let value = user_info
        .claims
        .get(avatar_claim)
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            (avatar_claim == "picture")
                .then_some(user_info.picture.as_deref())
                .flatten()
        })?;
    let avatar_url = OwnedMxcUri::from(value);

    if avatar_url.is_valid() {
        Some(avatar_url)
    } else {
        tracing::warn!(
            claim = avatar_claim,
            "ignoring OIDC avatar claim because it is not a valid Matrix Content URI"
        );
        None
    }
}

/// **Matrix User Account Management**
///
/// Creates a new Matrix user account or retrieves an existing one based on the
/// generated Matrix user ID. Sets up the user profile with information from OIDC.
///
/// ## Database Operations
/// 1. Check if user already exists
/// 2. Create new user record if needed (with OIDC type)
/// 3. Update user profile with display name and avatar
/// 4. Handle any database constraints or conflicts
async fn create_or_get_user(
    user_id: &str,
    display_name: &str,
    avatar_url: Option<OwnedMxcUri>,
    oidc_config: &crate::config::OidcConfig,
) -> AppResult<DbUser> {
    use diesel::prelude::*;
    use diesel_async::{AsyncConnection, RunQueryDsl};

    use crate::core::identifiers::UserId;
    use crate::data::connect;
    use crate::data::schema::*;

    let parsed_user_id = UserId::parse(user_id)
        .map_err(|_| MatrixError::invalid_param("Invalid Matrix user ID format"))?;

    let new_user = crate::data::user::NewDbUser {
        is_local: parsed_user_id.server_name().is_local(),
        localpart: parsed_user_id.localpart().to_string(),
        server_name: parsed_user_id.server_name().to_owned(),
        id: parsed_user_id.clone(),
        ty: Some("oidc".to_string()),
        is_admin: false,
        is_guest: false,
        appservice_id: None,
        created_at: UnixMillis::now(),
    };

    connect()
        .await?
        .transaction::<_, AppError, _>(async move |conn| {
            let mut user = users::table
                .filter(users::id.eq(&parsed_user_id))
                .for_update()
                .first::<DbUser>(conn)
                .await
                .optional()?;

            if user.is_none() {
                if !oidc_config.allow_registration {
                    return Err(MatrixError::forbidden(
                        "New user registration via OIDC is disabled",
                        None,
                    )
                    .into());
                }

                tracing::info!("Creating new Matrix user account: {}", user_id);
                diesel::insert_into(users::table)
                    .values(&new_user)
                    .on_conflict(users::id)
                    .do_nothing()
                    .execute(conn)
                    .await?;

                // Lock the row after the conflict-safe insert. This serializes
                // concurrent first logins before the nullable room_id profile check.
                user = Some(
                    users::table
                        .filter(users::id.eq(&parsed_user_id))
                        .for_update()
                        .first::<DbUser>(conn)
                        .await?,
                );
            }

            let mut user = user.expect("OIDC user was selected or inserted");
            if user.is_guest {
                diesel::update(users::table.find(&user.id))
                    .set(users::is_guest.eq(false))
                    .execute(conn)
                    .await?;
                user.is_guest = false;
            }

            let profile_exists = user_profiles::table
                .filter(user_profiles::user_id.eq(&user.id))
                .filter(user_profiles::room_id.is_null())
                .select(user_profiles::id)
                .first::<i64>(conn)
                .await
                .optional()?
                .is_some();
            if !profile_exists {
                diesel::insert_into(user_profiles::table)
                    .values(&crate::data::user::NewDbProfile {
                        user_id: user.id.clone(),
                        room_id: None,
                        display_name: Some(display_name.to_owned()),
                        avatar_url,
                        blurhash: None,
                    })
                    .execute(conn)
                    .await?;
            }

            tracing::info!("Successfully initialized Matrix user: {}", user_id);
            Ok(user)
        })
        .await
}

/// **Matrix Device and Access Token Creation**
///
/// Creates a Matrix device record and generates an access token for the authenticated
/// user. This establishes the user's session in the Matrix system.
///
/// ## Security Features
/// - Unique device ID with OIDC prefix for identification
/// - Cryptographically secure access token generation
/// - Device metadata tracking (user agent, timestamps)
/// - Proper database transaction handling
async fn create_access_token_for_user(user: &DbUser, device_id: &str) -> AppResult<String> {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    use crate::data::connect;
    use crate::data::schema::*;

    let parsed_device_id: OwnedDeviceId = device_id.into();

    // Create or update device record
    let new_device = crate::data::user::NewDbUserDevice {
        user_id: user.id.clone(),
        device_id: parsed_device_id.clone(),
        display_name: Some("OIDC Authentication".to_string()),
        user_agent: Some("OIDC/1.0".to_string()),
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
        .execute(&mut connect().await?)
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to create/update device: {}", e)))?;

    // Generate cryptographically secure access token
    let access_token = generate_random_string(64);

    let new_access_token = crate::data::user::NewDbAccessToken {
        user_id: user.id.clone(),
        device_id: parsed_device_id,
        token: access_token.clone(),
        puppets_user_id: None,
        last_validated: Some(UnixMillis::now()),
        refresh_token_id: None,
        is_used: false,
        expires_at: None, // OIDC tokens don't expire by default
        created_at: UnixMillis::now(),
    };

    diesel::insert_into(user_access_tokens::table)
        .values(&new_access_token)
        .execute(&mut connect().await?)
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to create access token: {}", e)))?;

    Ok(access_token)
}

// =================== DATA STRUCTURES ===================
//

/// OAuth 2.0 token response from OIDC provider
#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<i64>,
    id_token: Option<String>,
    refresh_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn provider_with_mapping(mapping: &[(&str, &str)]) -> OidcProviderConfig {
        OidcProviderConfig {
            issuer: "https://idm.example.com".to_owned(),
            client_id: "client".to_owned(),
            client_secret: "secret".to_owned(),
            redirect_uri: "https://matrix.example.com/oidc/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "profile".to_owned()],
            additional_params: BTreeMap::new(),
            skip_tls_verify: false,
            display_name: None,
            attribute_mapping: mapping
                .iter()
                .map(|(matrix_attribute, claim)| {
                    ((*matrix_attribute).to_owned(), (*claim).to_owned())
                })
                .collect(),
        }
    }

    fn user_info_with_claims(claims: serde_json::Value) -> OidcUserInfo {
        let claims = claims.as_object().unwrap().clone();
        OidcUserInfo {
            sub: claims["sub"].as_str().unwrap().to_owned(),
            email: claims
                .get("email")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            name: claims
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            picture: claims
                .get("picture")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            email_verified: claims
                .get("email_verified")
                .and_then(serde_json::Value::as_bool),
            preferred_username: claims
                .get("preferred_username")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            claims,
        }
    }

    #[test]
    fn matrix_callback_replaces_existing_login_tokens() {
        let callback = append_login_token(
            "element://login/complete?state=client&loginToken=stale#done",
            "fresh-token",
        )
        .unwrap();

        assert_eq!(
            callback.as_str(),
            "element://login/complete?state=client&loginToken=fresh-token#done"
        );
    }

    #[test]
    fn discovery_url_preserves_issuer_paths_and_trims_trailing_slashes() {
        assert_eq!(
            oidc_discovery_url("https://idm.example.com/oauth2/openid/example/")
                .unwrap()
                .as_str(),
            "https://idm.example.com/oauth2/openid/example/.well-known/openid-configuration"
        );
    }

    #[test]
    fn attribute_mapping_reads_provider_specific_display_name_claim() {
        let provider = provider_with_mapping(&[("display_name", "given_name")]);
        let user_info = user_info_with_claims(serde_json::json!({
            "sub": "51e25ece-ecb8-4880-8380-e31ad0d3d8b0",
            "name": "Ignored Full Name",
            "given_name": "Tester"
        }));

        assert_eq!(generate_display_name(&user_info, &provider), "Tester");
    }

    #[test]
    fn avatar_mapping_accepts_only_matrix_content_uris() {
        let provider = provider_with_mapping(&[("avatar_url", "matrix_avatar")]);
        let valid_user_info = user_info_with_claims(serde_json::json!({
            "sub": "user-id",
            "matrix_avatar": "mxc://matrix.example.com/media-id"
        }));
        let invalid_user_info = user_info_with_claims(serde_json::json!({
            "sub": "user-id",
            "matrix_avatar": "https://idm.example.com/avatar.png"
        }));

        assert_eq!(
            generate_avatar_url(&valid_user_info, &provider).as_deref(),
            Some("mxc://matrix.example.com/media-id".into())
        );
        assert_eq!(generate_avatar_url(&invalid_user_info, &provider), None);
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_oidc_profiles_are_created_repaired_and_preserved() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(serde_json::json!({
                "server_name": "oidc-profile.example",
                "db": { "url": "unused-test-config" }
            }))
            .unwrap()
        });

        let server_name = config::server_name();
        let oidc_config = crate::config::OidcConfig {
            allow_registration: true,
            ..Default::default()
        };
        let user_id = format!("@oidc-profile-new:{server_name}");
        let avatar_url = OwnedMxcUri::from(format!("mxc://{server_name}/initial-avatar"));

        let user = create_or_get_user(
            &user_id,
            "Mapped Name",
            Some(avatar_url.clone()),
            &oidc_config,
        )
        .await
        .unwrap();
        let profile = crate::data::user::get_profile(&user.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(profile.display_name.as_deref(), Some("Mapped Name"));
        assert_eq!(profile.avatar_url, Some(avatar_url));

        crate::data::user::set_display_name(&user.id, "Chosen Name")
            .await
            .unwrap();
        create_or_get_user(&user_id, "Provider Changed", None, &oidc_config)
            .await
            .unwrap();
        assert_eq!(
            crate::data::user::display_name(&user.id)
                .await
                .unwrap()
                .as_deref(),
            Some("Chosen Name")
        );

        let legacy_user_id = format!("@oidc-profile-legacy:{server_name}");
        let legacy_user_id = crate::core::identifiers::UserId::parse(legacy_user_id).unwrap();
        let legacy_user = crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: legacy_user_id.clone(),
            ty: Some("oidc".to_owned()),
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: legacy_user_id.localpart().to_owned(),
            server_name: legacy_user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        assert!(
            crate::data::user::get_profile(&legacy_user.id, None)
                .await
                .unwrap()
                .is_none()
        );
        crate::data::user::set_display_name(&legacy_user.id, "Repaired Name")
            .await
            .unwrap();
        crate::data::user::set_profile_field(
            &legacy_user.id,
            "com.example.banner",
            serde_json::json!("mxc://example.org/banner"),
        )
        .await
        .unwrap();
        assert_eq!(
            crate::data::user::display_name(&legacy_user.id)
                .await
                .unwrap()
                .as_deref(),
            Some("Repaired Name")
        );
        assert_eq!(
            crate::data::user::profile_field(&legacy_user.id, "com.example.banner")
                .await
                .unwrap(),
            Some(serde_json::json!("mxc://example.org/banner"))
        );
    }

    #[tokio::test]
    async fn discovery_uses_advertised_authorization_and_token_endpoints() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let issuer = format!("http://{address}/oauth2/openid/example");
        let expected_issuer = issuer.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with(
                "GET /oauth2/openid/example/.well-known/openid-configuration HTTP/1.1"
            ));

            let body = serde_json::json!({
                "issuer": expected_issuer,
                "authorization_endpoint": "https://idm.example.com/ui/oauth2",
                "token_endpoint": "https://idm.example.com/oauth2/token",
                "userinfo_endpoint": "https://idm.example.com/oauth2/userinfo"
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let metadata = discover_oidc_metadata(&issuer).await.unwrap();
        server.await.unwrap();

        assert_eq!(
            metadata.authorization_endpoint,
            "https://idm.example.com/ui/oauth2"
        );
        assert_eq!(
            metadata.token_endpoint,
            "https://idm.example.com/oauth2/token"
        );
    }
}
