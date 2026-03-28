use serde::Deserialize;

use crate::macros::config_example;

fn default_introspection_cache_ttl() -> u64 {
    300
}

#[config_example(filename = "palpo-example.toml", section = "delegated_auth")]
#[derive(Clone, Debug, Deserialize, Default)]
pub struct DelegatedAuthConfig {
    /// Enable MSC3861 delegated OIDC authentication.
    /// When enabled, Palpo delegates token validation to an external
    /// authorization server (like Pasion) via token introspection.
    ///
    /// default: false
    #[serde(default)]
    pub enable: bool,

    /// The issuer URL of the authorization server (e.g. "http://localhost:8080/").
    /// Used in well-known responses and auth_metadata.
    pub issuer: Option<String>,

    /// The token introspection endpoint URL (RFC 7662).
    /// Defaults to "{issuer}/oauth2/introspect" if not set.
    pub introspection_endpoint: Option<String>,

    /// The OAuth2 client_id that Palpo uses when redirecting users to the
    /// authorization server for SSO login.
    pub client_id: Option<String>,

    /// Optional URL for account management UI.
    /// Included in the well-known client response under m.authentication.
    pub account_management_url: Option<String>,

    /// Cache TTL for introspection results in seconds.
    /// Set to 0 to disable caching.
    ///
    /// default: 300
    #[serde(default = "default_introspection_cache_ttl")]
    pub introspection_cache_ttl: u64,
}
