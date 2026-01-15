use std::collections::BTreeMap;

use serde::Deserialize;

use crate::core::serde::default_true;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "oidc")]
#[derive(Clone, Debug, Deserialize, Default)]
pub struct OidcConfig {
    /// Enable OIDC/OAuth authentication
    ///
    /// Allows users to sign in using external providers (Google, GitHub, etc.)
    /// instead of Matrix passwords
    ///
    /// default: false
    #[serde(default)]
    pub enable: bool,

    /// Provider configurations
    ///
    /// Map of provider name to configuration. Each provider needs:
    /// - issuer: Provider base URL
    /// - client_id: OAuth app ID
    /// - client_secret: OAuth app secret
    /// - redirect_uri: Callback URL (must match provider settings)
    /// - scopes (optional): Permissions to request
    /// - display_name (optional): UI display text
    ///
    /// GitHub example:
    /// ```toml
    /// [oidc.providers.github]
    /// issuer = "https://github.com"
    /// client_id = "your_app_id"
    /// client_secret = "your_secret"
    /// redirect_uri = "https://server/_matrix/client/oidc/callback"
    /// scopes = ["read:user", "user:email"]
    /// ```
    ///
    /// default: {}
    #[serde(default)]
    pub providers: BTreeMap<String, OidcProviderConfig>,

    /// Default provider name
    ///
    /// Used when accessing /oidc/auth without specifying provider
    ///
    /// example: "github"
    /// default: None (first alphabetically)
    pub default_provider: Option<String>,

    /// Auto-create new users on first login
    ///
    /// When true: New accounts created automatically
    /// When false: Only existing Matrix users can use OAuth login
    ///
    /// default: true
    #[serde(default = "default_true")]
    pub allow_registration: bool,

    /// User ID generation strategy (deprecated - auto-detected now)
    ///
    /// The system now automatically chooses the best identifier:
    /// 1. Username (GitHub login, preferred_username)
    /// 2. Email prefix (john from john@example.com)
    /// 3. Provider ID with "user" prefix
    ///
    /// This field is kept for backwards compatibility
    ///
    /// default: "email"
    #[serde(default = "default_user_mapping")]
    pub user_mapping: String,

    /// Prefix for OAuth user IDs
    ///
    /// Adds prefix to distinguish OAuth users from regular Matrix users
    /// Empty string for cleaner usernames
    ///
    /// example: "gh_" → @gh_username:server
    /// default: ""
    #[serde(default)]
    pub user_prefix: String,

    /// Require verified email for login
    ///
    /// Set to false for GitHub users with private emails
    /// Set to true for providers where email verification is critical
    ///
    /// default: true
    #[serde(default = "default_true")]
    pub require_email_verified: bool,

    /// OAuth session timeout (seconds)
    ///
    /// Time limit for completing the OAuth flow
    ///
    /// default: 600 (10 minutes)
    #[serde(default = "default_session_timeout")]
    pub session_timeout: u64,

    /// Enable PKCE security extension
    ///
    /// Adds extra security to OAuth flow (recommended)
    ///
    /// default: true
    #[serde(default = "default_true")]
    pub enable_pkce: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OidcProviderConfig {
    /// Provider base URL
    ///
    /// OAuth provider's base URL (e.g., "https://github.com")
    pub issuer: String,

    /// OAuth app client ID
    ///
    /// Get this from your OAuth app settings
    pub client_id: String,

    /// OAuth app client secret
    ///
    /// Keep this secure - never commit to version control
    pub client_secret: String,

    /// Callback URL after authentication
    ///
    /// Must exactly match the URL in your OAuth app settings
    /// Format: "https://your-server/_matrix/client/oidc/callback"
    pub redirect_uri: String,

    /// Permissions to request from provider
    ///
    /// GitHub: ["read:user", "user:email"]
    /// Google: ["openid", "email", "profile"] (default)
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,

    /// Extra OAuth parameters (optional)
    ///
    /// Provider-specific parameters
    /// example: { "prompt" = "select_account" }
    ///
    /// default: {}
    #[serde(default)]
    pub additional_params: BTreeMap<String, String>,

    /// Skip TLS verification (DEV ONLY - INSECURE)
    ///
    /// default: false
    #[serde(default)]
    pub skip_tls_verify: bool,

    /// UI display text for this provider
    ///
    /// example: "Sign in with GitHub"
    /// default: Provider name
    pub display_name: Option<String>,

    /// Custom attribute mapping
    ///
    /// Override the default mapping of OIDC claims to Matrix user attributes.
    /// Keys are Matrix attributes, values are OIDC claim names.
    ///
    /// example: { "display_name" = "given_name", "avatar_url" = "picture" }
    ///
    /// default: {}
    #[serde(default)]
    pub attribute_mapping: BTreeMap<String, String>,
}

fn default_user_mapping() -> String {
    "email".to_string()
}

fn default_session_timeout() -> u64 {
    600 // 10 minutes
}

fn default_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ]
}
