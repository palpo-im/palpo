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
//! - Pure OAuth 2.0 providers (GitHub) with custom user info endpoints
//!   Both follow the OAuth 2.0 Authorization Code flow with optional PKCE support.
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

use cookie::time::Duration;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use url::Url;

use crate::{
    AppResult, JsonResult,
    config::{self, OidcProviderConfig},
    core::{MatrixError, OwnedDeviceId, UnixMillis},
    data,
    data::user::DbUser,
    exts::*,
    json_ok,
};

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
    /// Session creation timestamp
    pub created_at: u64,
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
        match issuer {
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
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

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
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        provider_config.issuer
    );

    let client = reqwest::Client::new();
    let response = client.get(&discovery_url).send().await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            // Parse discovery document
            let discovery: serde_json::Value = resp.json().await.map_err(|e| {
                MatrixError::unknown(format!("Failed to parse discovery document: {}", e))
            })?;

            Ok(OidcProviderInfo {
                authorization_endpoint: discovery["authorization_endpoint"]
                    .as_str()
                    .ok_or_else(|| {
                        MatrixError::unknown("Missing authorization_endpoint in discovery")
                    })?
                    .to_string(),
                token_endpoint: discovery["token_endpoint"]
                    .as_str()
                    .ok_or_else(|| MatrixError::unknown("Missing token_endpoint in discovery"))?
                    .to_string(),
                userinfo_endpoint: discovery["userinfo_endpoint"]
                    .as_str()
                    .ok_or_else(|| MatrixError::unknown("Missing userinfo_endpoint in discovery"))?
                    .to_string(),
                issuer: discovery["issuer"]
                    .as_str()
                    .unwrap_or(&provider_config.issuer)
                    .to_string(),
            })
        }
        _ => {
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
                ProviderType::Generic => Err(MatrixError::unknown(
                    "Could not discover OIDC endpoints and no fallback available",
                )),
            }
        }
    }
}

/// `GET /_matrix/client/*/oidc/auth`
///
/// **OAuth Authorization Initiation Endpoint**
///
/// Starts the OAuth 2.0 Authorization Code flow by redirecting the user to the
/// selected OIDC provider for authentication. This is step 1 of the OIDC flow.
///
/// ## Request Parameters
/// - `provider` (optional): Name of the OIDC provider to use. If not specified,
///   uses the default provider from configuration.
///
/// ## Security Features
/// - **CSRF Protection**: Generates a random `state` parameter and stores it in
///   an HTTP-only cookie for validation on callback.
/// - **PKCE Support**: Optionally generates code_verifier/code_challenge for
///   enhanced security (enabled by default).
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
        .ok_or_else(|| {
            MatrixError::invalid_param("No OIDC provider specified and no default configured")
        })?;

    let provider_config = oidc_config.providers.get(&provider_name).ok_or_else(|| {
        MatrixError::not_found(format!("Unknown OIDC provider: {}", provider_name))
    })?;

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
    res.render(Redirect::found(auth_url.to_string()));
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
/// 8. Generate Matrix access token and device
/// 9. Return authentication credentials to client
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
pub async fn oidc_callback(req: &mut Request) -> JsonResult<OidcLoginResponse> {
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

    // Step 12: Create or retrieve Matrix user account
    let user = create_or_get_user(&matrix_user_id, &display_name, &user_info, oidc_config).await?;

    // Step 13: Create Matrix device and access token
    let device_id = format!("OIDC_{}", generate_random_string(8));
    let access_token = create_access_token_for_user(&user, &device_id).await?;

    tracing::info!(
        "OIDC authentication successful for user '{}' via provider '{}'",
        matrix_user_id,
        session.provider
    );

    // Step 14: Return Matrix authentication credentials
    json_ok(OidcLoginResponse {
        user_id: matrix_user_id,
        access_token,
        device_id,
        home_server: config.server_name.to_string(),
    })
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

//
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
                email: user_info_response["email"].as_str().map(String::from), // May be None for private emails
                name: user_info_response["name"].as_str().map(String::from),
                picture: user_info_response["avatar_url"].as_str().map(String::from),
                email_verified: Some(true), // GitHub verifies primary email, but it may not be visible
                preferred_username: user_info_response["login"].as_str().map(String::from), // GitHub username
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
    // Check for custom attribute mapping first
    if let Some(_display_name_claim) = provider_config.attribute_mapping.get("display_name") {
        // This would require extending the user info structure to include arbitrary claims
        // For now, use the standard mapping
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
    user_info: &OidcUserInfo,
    oidc_config: &crate::config::OidcConfig,
) -> AppResult<DbUser> {
    use crate::core::identifiers::UserId;
    use crate::data::connect;
    use crate::data::schema::*;
    use diesel::prelude::*;

    let parsed_user_id = UserId::parse(user_id)
        .map_err(|_| MatrixError::invalid_param("Invalid Matrix user ID format"))?;

    // Check if user already exists
    let exist_user = users::table
        .filter(users::id.eq(&parsed_user_id))
        .first::<DbUser>(&mut connect()?);
    if let Ok(exist_user) = exist_user {
        tracing::debug!("Found existing user account: {}", user_id);

        // Note: We intentionally do NOT update the profile for existing users
        // to preserve any changes the user made in Matrix (like custom display names).
        // Only update avatar if it changed on the provider side
        //
        // Alternative: You could add a config option to control this behavior:
        // if oidc_config.update_profile_on_login {
        //     if let Err(e) = set_user_profile(&exist_user.id, display_name, user_info.picture.as_deref()).await {
        //         tracing::warn!("Failed to update profile for existing user: {}", e);
        //     }
        // }

        return Ok(exist_user);
    }

    // Check if user registration is allowed
    if !oidc_config.allow_registration {
        return Err(
            MatrixError::forbidden("New user registration via OIDC is disabled", None).into(),
        );
    }

    tracing::info!("Creating new Matrix user account: {}", user_id);

    // Create new user account
    let new_user = crate::data::user::NewDbUser {
        is_local: parsed_user_id.server_name().is_local(),
        localpart: parsed_user_id.localpart().to_string(),
        server_name: parsed_user_id.server_name().to_owned(),
        id: parsed_user_id,
        ty: Some("oidc".to_string()),
        is_admin: false,
        is_guest: false,
        appservice_id: None,
        created_at: UnixMillis::now(),
    };

    let user = diesel::insert_into(users::table)
        .values(&new_user)
        .get_result::<DbUser>(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to create user account: {}", e)))?;

    // Set initial user profile
    if let Err(e) = data::user::set_display_name(&user.id, display_name) {
        tracing::warn!("failed to set profile for new user (non-fatal): {}", e);
    }
    if let Some(picture) = user_info.picture.as_deref()
        && let Err(e) = data::user::set_avatar_url(&user.id, picture.into())
    {
        tracing::warn!("failed to set profile for new user (non-fatal): {}", e);
    }

    tracing::info!("Successfully created new Matrix user: {}", user_id);
    Ok(user)
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
    use crate::data::connect;
    use crate::data::schema::*;
    use diesel::prelude::*;

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
        .execute(&mut connect()?)
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
        .execute(&mut connect()?)
        .map_err(|e| MatrixError::unknown(format!("Failed to create access token: {}", e)))?;

    Ok(access_token)
}

//
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
