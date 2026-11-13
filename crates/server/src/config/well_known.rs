use serde::Deserialize;
use url::Url;

use crate::core::client::discovery::client::RtcFocusInfo;
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
    /// If not configured, defaults to server_name with port 443.
    ///
    /// Format: "hostname:port" (e.g., "matrix.example.com:443")
    /// Note: Only hostname and port, no protocol scheme. ALWAYS include the port:
    /// a value without one makes other homeservers fall back to port 8448 (per the
    /// Matrix server-resolution spec), which is almost never what you want behind a
    /// reverse proxy on 443.
    ///
    /// Example scenarios:
    /// - Behind proxy on 443: server = "matrix.example.com:443"
    /// - Direct deployment on the standard federation port: server = "matrix.example.com:8448"
    /// - Different federation port: server = "matrix.example.com:9008"
    ///
    /// Example: "matrix.palpo.im:443"
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

    /// URI to a PGP key for the support contact.
    ///
    /// This is exposed as `dev.zirco.msc4439.pgp_key` in `/.well-known/matrix/support`.
    ///
    /// Example: "https://example.com/security.asc"
    pub support_pgp_key: Option<String>,

    /// MatrixRTC foci advertised in `/.well-known/matrix/client` (MSC4143), ordered
    /// by priority. Element Call and other MatrixRTC clients read this to find the
    /// LiveKit JWT service (e.g. lk-jwt-service) used for voice/video calls.
    ///
    /// Leave empty (the default) to advertise no foci.
    ///
    /// Example:
    ///
    /// ```toml
    /// [[well_known.rtc_foci]]
    /// type = "livekit"
    /// livekit_service_url = "https://livekit-jwt.example.com"
    /// ```
    ///
    /// Each entry may also set `url` (the LiveKit SFU WebSocket URL).
    #[serde(default)]
    pub rtc_foci: Vec<RtcFocusInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::client::discovery::client::LiveKitRtcFocusInfo;

    #[test]
    fn rtc_foci_defaults_to_empty() {
        let conf: WellKnownConfig = toml::from_str("").unwrap();
        assert!(conf.rtc_foci.is_empty());
    }

    // The JWT-service form (`livekit_service_url`) is what Element Call reads today.
    #[allow(deprecated)]
    fn legacy_livekit(service_url: &str) -> RtcFocusInfo {
        RtcFocusInfo::LiveKit(LiveKitRtcFocusInfo::with_service_url(service_url.to_owned()))
    }

    #[test]
    fn rtc_foci_parses_livekit_focus() {
        let conf: WellKnownConfig = toml::from_str(
            r#"
            [[rtc_foci]]
            type = "livekit"
            livekit_service_url = "http://127.0.0.1:8071"
            "#,
        )
        .unwrap();
        assert_eq!(
            conf.rtc_foci,
            vec![legacy_livekit("http://127.0.0.1:8071")]
        );
    }

    #[test]
    fn rtc_foci_serialize_under_msc4143_key() {
        use crate::core::client::discovery::client::{ClientResBody, HomeServerInfo};
        let mut body = ClientResBody::new(HomeServerInfo::new("http://hs".to_owned()));
        body.rtc_foci = vec![legacy_livekit("http://jwt")];
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json["org.matrix.msc4143.rtc_foci"],
            serde_json::json!([{"type": "livekit", "livekit_service_url": "http://jwt"}])
        );
    }
}
