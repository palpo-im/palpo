use serde::Deserialize;

use crate::core::ServerName;
use crate::core::serde::default_true;
use crate::macros::config_example;

use super::WildCardedDomain;

#[config_example(filename = "palpo-example.toml", section = "federation")]
#[derive(Clone, Debug, Deserialize)]
pub struct FederationConfig {
    /// Controls whether federation is allowed or not. It is not recommended to
    /// disable this after the fact due to potential federation breakage.
    #[serde(default = "default_true")]
    pub enable: bool,

    /// Allows federation requests to be made to itself
    ///
    /// This isn't intended and is very likely a bug if federation requests are
    /// being sent to yourself. This currently mainly exists for development
    /// purposes.
    #[serde(default)]
    pub allow_loopback: bool,

    /// Set this to true to allow federating device display names / allow
    /// external users to see your device display name. If federation is
    /// disabled entirely (`allow_federation`), this is inherently false. For
    /// privacy reasons, this is best left disabled.
    #[serde(default)]
    pub allow_device_name: bool,

    /// Config option to allow or disallow incoming federation requests that
    /// obtain the profiles of our local users from
    /// `/_matrix/federation/v1/query/profile`
    ///
    /// Increases privacy of your local user's such as display names, but some
    /// remote users may get a false "this user does not exist" error when they
    /// try to invite you to a DM or room. Also can protect against profile
    /// spiders.
    ///
    /// This is inherently false if `allow_federation` is disabled
    #[serde(default = "default_true")]
    pub allow_inbound_profile_lookup: bool,

    /// Allowlist of servers permitted to federate. If set, only servers
    /// matching these patterns can communicate. Supports wildcards: `*.example.com`.
    /// When `None`, all servers are allowed (subject to `denied_servers`).
    #[serde(default)]
    pub allowed_servers: Option<Vec<WildCardedDomain>>,

    /// Denylist of servers blocked from federation. Takes precedence over
    /// `allowed_servers`. Supports wildcards: `*.evil.com`.
    #[serde(default)]
    pub denied_servers: Vec<WildCardedDomain>,
}

impl FederationConfig {
    /// Check if a remote server is allowed to federate with this server.
    /// Denied servers are checked first (deny takes precedence), then
    /// allowed servers (if configured).
    pub fn is_server_allowed(&self, server: &ServerName) -> bool {
        if !self.enable {
            return false;
        }
        let host = server.host();
        // Deny list takes precedence
        if self.denied_servers.iter().any(|d| d.matches(host)) {
            return false;
        }
        // If allow list is set, server must match at least one pattern
        if let Some(ref allowed) = self.allowed_servers {
            return allowed.iter().any(|a| a.matches(host));
        }
        true
    }
}

impl Default for FederationConfig {
    fn default() -> Self {
        Self {
            enable: true,
            allow_loopback: false,
            allow_device_name: false,
            allow_inbound_profile_lookup: true,
            allowed_servers: None,
            denied_servers: Vec::new(),
        }
    }
}
