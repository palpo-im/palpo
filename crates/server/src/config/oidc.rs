use std::collections::BTreeMap;
use std::fmt;

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

    /// Trusted Matrix clients allowed to receive a local OIDC SSO login token.
    /// HTTP(S) entries must be origins, for example "https://chat.example.org".
    /// A loopback origin without a port permits ephemeral client ports.
    /// Native clients must register their complete callback URI, for example
    /// "element://vector/webapp"; its path must match exactly. Query parameters
    /// may carry per-login client state. Unknown destinations are rejected.
    ///
    /// default: []
    #[serde(default)]
    pub sso_allowed_redirect_origins: Vec<String>,

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

    /// External MAS (Matrix Authentication Service) issuer URL
    ///
    /// When set, Palpo advertises this URL as the OIDC issuer in
    /// .well-known/matrix/client (m.authentication), enabling Element X
    /// and other MSC3861-compatible clients to use the external MAS
    /// for native OIDC login.
    ///
    /// This field is independent of `enable` above. Even with
    /// enable = false (legacy OIDC client disabled), setting mas_issuer
    /// will still advertise the issuer in .well-known discovery.
    ///
    /// example: "https://auth.example.com/"
    pub mas_issuer: Option<String>,
}

/// Match an explicitly trusted SSO client. HTTP origins trust the client at
/// every path; native applications register an exact callback path instead of
/// granting an entire custom scheme. Never match opaque or credentialed URLs.
pub(crate) fn sso_redirect_allowed(client: &url::Url, allowed: &[String]) -> bool {
    if !client.username().is_empty() || client.password().is_some() || client.host().is_none() {
        return false;
    }
    allowed.iter().any(|entry| {
        let Ok(trusted) = url::Url::parse(entry) else {
            return false;
        };
        if !trusted.username().is_empty()
            || trusted.password().is_some()
            || trusted.host().is_none()
            || trusted.query().is_some()
            || trusted.fragment().is_some()
            || client.scheme() != trusted.scheme()
            || client.host() != trusted.host()
        {
            return false;
        }
        if matches!(trusted.scheme(), "http" | "https") {
            if trusted.path() != "/" {
                return false;
            }
            let loopback = matches!(client.host_str(), Some("127.0.0.1" | "localhost"))
                || matches!(client.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
            // Url normalizes explicit :80/:443 to no port. Inspect the
            // configured authority before granting the ephemeral-port exception.
            // Non-origin spellings that Uri cannot parse do not get a wildcard.
            let portless_loopback = loopback
                && entry
                    .parse::<salvo::http::uri::Uri>()
                    .ok()
                    .is_some_and(|uri| {
                        uri.authority()
                            .is_some_and(|authority| authority.port().is_none())
                    });
            portless_loopback || client.port_or_known_default() == trusted.port_or_known_default()
        } else {
            !matches!(
                trusted.scheme(),
                "file" | "javascript" | "data" | "blob" | "about"
            ) && client.path() == trusted.path()
                && client.port() == trusted.port()
                && client.fragment().is_none()
        }
    })
}

#[derive(Clone, Deserialize)]
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
    /// The mapped avatar claim may contain an `mxc://` Matrix Content URI or
    /// an HTTPS URL for a PNG, JPEG, GIF, or WebP image. HTTPS images are
    /// fetched and stored as Matrix media when a user's profile is first
    /// created. Existing avatars are preserved.
    ///
    /// example: { "display_name" = "given_name", "avatar_url" = "picture" }
    ///
    /// default: {}
    #[serde(default)]
    pub attribute_mapping: BTreeMap<String, String>,
}

// Custom Debug implementation to prevent leaking client_secret
impl fmt::Debug for OidcProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidcProviderConfig")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("redirect_uri", &self.redirect_uri)
            .field("scopes", &self.scopes)
            .field("additional_params", &self.additional_params)
            .field("skip_tls_verify", &self.skip_tls_verify)
            .field("display_name", &self.display_name)
            .field("attribute_mapping", &self.attribute_mapping)
            .finish()
    }
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

#[cfg(test)]
mod sso_redirect_policy_tests {
    fn accepts(origin: &str, destination: &str) -> bool {
        super::sso_redirect_allowed(&url::Url::parse(destination).unwrap(), &[origin.to_owned()])
    }

    #[test]
    fn explicit_loopback_ports_preserve_default_port_restrictions() {
        for (origin, destination, other_port) in [
            (
                "http://127.0.0.1:80",
                "http://127.0.0.1/callback",
                "http://127.0.0.1:49152/callback",
            ),
            (
                "http://localhost:80",
                "http://localhost/callback",
                "http://localhost:49152/callback",
            ),
            (
                "http://[::1]:80",
                "http://[::1]/callback",
                "http://[::1]:49152/callback",
            ),
            (
                "https://127.0.0.1:443",
                "https://127.0.0.1/callback",
                "https://127.0.0.1:49152/callback",
            ),
            (
                "https://localhost:443",
                "https://localhost/callback",
                "https://localhost:49152/callback",
            ),
            (
                "https://[::1]:443",
                "https://[::1]/callback",
                "https://[::1]:49152/callback",
            ),
            (
                "http://127.0.0.1:8080",
                "http://127.0.0.1:8080/callback",
                "http://127.0.0.1/callback",
            ),
            (
                "http://[::1]:8080",
                "http://[::1]:8080/callback",
                "http://[::1]/callback",
            ),
        ] {
            assert!(
                accepts(origin, destination),
                "{origin} must accept {destination}"
            );
            assert!(
                !accepts(origin, other_port),
                "{origin} must reject {other_port}"
            );
        }
    }

    #[test]
    fn portless_loopback_and_non_loopback_origins_keep_their_scope() {
        for origin in [
            "http://127.0.0.1",
            "http://localhost",
            "http://[::1]",
            "https://127.0.0.1",
            "https://localhost",
            "https://[::1]",
        ] {
            assert!(
                accepts(origin, &format!("{origin}:49152/callback")),
                "{origin}"
            );
        }
        for origin in ["http://client.example", "https://client.example"] {
            assert!(accepts(origin, &format!("{origin}/callback")), "{origin}");
            assert!(
                !accepts(origin, &format!("{origin}:49152/callback")),
                "{origin}"
            );
        }
        // An explicit default port must also stay strict when zero-padded.
        assert!(!accepts(
            "http://127.0.0.1:00080",
            "http://127.0.0.1:49152/callback"
        ));
        assert!(!accepts(
            "https://[::1]:00443",
            "https://[::1]:49152/callback"
        ));
    }
}
