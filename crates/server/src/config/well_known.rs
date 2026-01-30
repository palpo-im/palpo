use serde::Deserialize;
use url::Url;

use crate::core::client::discovery::support::ContactRole;
use crate::core::{OwnedServerName, OwnedUserId};
use crate::macros::config_example;

/// Configure Matrix well-known endpoints for service discovery.
///
/// Palpo serves the /.well-known/matrix/* endpoints, which allow clients and other
/// servers to discover how to connect to this server.
///
/// These endpoints are essential for Matrix federation and client connectivity:
/// - `/.well-known/matrix/server` - Server discovery for federation
/// - `/.well-known/matrix/client` - Client discovery for homeserver API
///
/// For more information, see the Matrix specification on server discovery.
#[config_example(filename = "palpo-example.toml", section = "well_known")]
#[derive(Clone, Debug, Deserialize, Default)]
pub struct WellKnownConfig {
    /// Client discovery endpoint (/.well-known/matrix/client)
    ///
    /// Used by Matrix clients (Element, FluffyChat, etc.) to find the correct
    /// homeserver URL for user API calls (login, sync, sending messages, etc.).
    ///
    /// If not configured, defaults to using server_name with port 443 (HTTPS).
    ///
    /// IMPORTANT: Set this manually when Palpo is behind a reverse proxy, load balancer,
    /// or CDN. The URL should be the external address that clients can reach.
    ///
    /// Format: Full URL with scheme (e.g., "https://matrix.example.com")
    ///
    /// Example scenarios:
    /// - Direct deployment: client = "https://matrix.example.com"
    /// - Behind proxy: client = "https://matrix.example.com" (proxy on port 443)
    /// - With CDN: client = "https://cdn.example.com"
    /// - Custom domain: client = "https://chat.example.com"
    /// - Local testing: client = "http://localhost:8080"
    ///
    /// Example: "https://matrix.palpo.im"
    pub client: Option<String>,

    /// Server discovery endpoint (/.well-known/matrix/server)
    ///
    /// Used by other Matrix servers (federation) to find the correct endpoint for
    /// federated communication. This is the address that other homeservers will use
    /// to connect to your server.
    ///
    /// If not configured, defaults to server_name with port 8448 (standard federation port).
    ///
    /// Format: "hostname:port" (e.g., "matrix.example.com:8448")
    /// Note: Only hostname and port, no protocol scheme.
    ///
    /// Example scenarios:
    /// - Direct deployment: server = "matrix.example.com:8448"
    /// - Behind proxy: server = "matrix.example.com:8448" (proxy handles the routing)
    /// - Different federation port: server = "matrix.example.com:9008"
    ///
    /// Example: "matrix.palpo.im:8448"
    pub server: Option<OwnedServerName>,

    /// URL to a support page for this Matrix server. This can be included
    /// in the server's support contact information to help users find
    /// assistance or documentation.
    ///
    /// Example: "https://example.com/matrix-support"
    pub support_page: Option<Url>,

    /// The role/type of the support contact. This helps categorize what
    /// kind of support is available (e.g. administrator, security contact).
    ///
    /// Common values include roles like "m.role.admin" or "m.role.security"
    pub support_role: Option<ContactRole>,

    /// Email address for server support contact. Users and other server
    /// administrators can use this to reach out for help or report issues.
    ///
    /// Example: "admin@example.com"
    pub support_email: Option<String>,

    /// Matrix User ID (MXID) of a support contact on this server.
    /// This provides an in-Matrix way for users to contact support.
    ///
    /// Example: "@admin:example.com"
    pub support_mxid: Option<OwnedUserId>,
}
