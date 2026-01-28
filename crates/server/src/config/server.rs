use std::collections::BTreeMap;
use std::path::PathBuf;

use regex::RegexSet;
use salvo::http::HeaderValue;
use serde::Deserialize;
use serde::de::IgnoredAny;
use url::Url;

use super::{
    AdminConfig, BlurhashConfig, CompressionConfig, DbConfig, FederationConfig, HttpClientConfig,
    JwtConfig, LoggerConfig, MediaConfig, OidcConfig, PresenceConfig, ProxyConfig,
    ReadReceiptConfig, TurnConfig, TypingConfig, UrlPreviewConfig,
};
use crate::core::client::discovery::support::ContactRole;
use crate::core::serde::{default_false, default_true};
use crate::core::{OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomVersionId};
use crate::env_vars::required_var;
use crate::macros::config_example;
use crate::utils::sys;
use crate::{AppError, AppResult};

const DEPRECATED_KEYS: &[&str; 0] = &[];

#[derive(Clone, Debug, Deserialize, Default)]
pub struct WellKnownConfig {
    pub client: Option<String>,
    pub server: Option<OwnedServerName>,
    pub support_page: Option<Url>,
    pub support_role: Option<ContactRole>,
    pub support_email: Option<String>,
    pub support_mxid: Option<OwnedUserId>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct KeypairConfig {
    pub document: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListenerConfig {
    /// The default address (IPv4 or IPv6) and port palpo will listen on.
    #[serde(default = "default_listen_address")]
    pub address: String,
    #[serde(default)]
    pub x_forwarded: bool,
    // external structure; separate section
    pub tls: Option<TlsConfig>,
}
impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            address: default_listen_address(),
            x_forwarded: false,
            tls: None,
        }
    }
}
impl ListenerConfig {
    pub fn enabled_tls(&self) -> Option<&TlsConfig> {
        if let Some(tls) = self.tls.as_ref() {
            if tls.enable { Some(tls) } else { None }
        } else {
            None
        }
    }
}

#[config_example(
    filename = "palpo-example.toml",
    undocumented = "# This item is undocumented. Please contribute documentation for it.",
    header = r#"### Palpo Configuration
###
### THIS FILE IS GENERATED. CHANGES/CONTRIBUTIONS IN THE REPO WILL BE
### OVERWRITTEN!
###
### You should rename this file before configuring your server. Changes to
### documentation and defaults can be contributed in source code at
### crate/server/config/server.rs. This file is generated when building.
###
### Any values pre-populated are the default values for said config option.
###
### At the minimum, you MUST edit all the config options to your environment
### that say "YOU NEED TO EDIT THIS".
###
### For more information, see:
### https://palpo.im/guide/configuration.html
"#,
    ignore = "catch_others federation well_known compression typing read_receipt presence \
        admin url_preview turn media blurhash keypair ldap proxy jwt oidc logger db appservice"
)]
#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listener")]
    pub listeners: Vec<ListenerConfig>,

    /// The server_name is the pretty name of this server. It is used as a
    /// suffix for user and room IDs/aliases.
    /// YOU NEED TO EDIT THIS.
    ///
    /// example: "palpo.im"
    #[serde(default = "default_server_name")]
    pub server_name: OwnedServerName,

    pub home_page: Option<String>,

    // display: hidden
    #[serde(default)]
    pub db: DbConfig,

    // display: hidden
    #[serde(default = "default_true")]
    pub allow_check_for_updates: bool,
    #[serde(default = "default_max_concurrent_requests")]
    pub max_concurrent_requests: u16,

    /// Text which will be added to the end of the user's displayname upon
    /// registration with a space before the text. In Conduit, this was the
    /// lightning bolt emoji.
    ///
    /// To disable, set this to "" (an empty string).
    ///
    /// default: "💕"
    #[serde(default = "default_new_user_displayname_suffix")]
    pub new_user_displayname_suffix: String,

    // /// The UNIX socket palpo will listen on.
    // ///
    // /// palpo cannot listen on both an IP address and a UNIX socket. If
    // /// listening on a UNIX socket, you MUST remove/comment the `address` key.
    // ///
    // /// Remember to make sure that your reverse proxy has access to this socket
    // /// file, either by adding your reverse proxy to the 'palpo' group or
    // /// granting world R/W permissions with `unix_socket_perms` (666 minimum).
    // ///
    // /// example: "/run/palpo/palpo.sock"
    // pub unix_socket_path: Option<PathBuf>,

    // /// The default permissions (in octal) to create the UNIX socket with.
    // ///
    // /// default: 660
    // #[serde(default = "default_unix_socket_perms")]
    // pub unix_socket_perms: u32,
    /// Enable to query all nameservers until the domain is found. Referred to
    /// as "trust_negative_responses" in hickory_resolver. This can avoid
    /// useless DNS queries if the first nameserver responds with NXDOMAIN or
    /// an empty NOERROR response.
    #[serde(default = "default_true")]
    pub query_all_nameservers: bool,

    /// Enable using *only* TCP for querying your specified nameservers instead
    /// of UDP.
    ///
    /// If you are running palpo in a container environment, this config
    /// option may need to be enabled. For more details, see:
    /// https://palpo.im/troubleshooting.html#potential-dns-issues-when-using-docker
    #[serde(default)]
    pub query_over_tcp_only: bool,

    /// DNS A/AAAA record lookup strategy
    ///
    /// Takes a number of one of the following options:
    /// 1 - Ipv4Only (Only query for A records, no AAAA/IPv6)
    ///
    /// 2 - Ipv6Only (Only query for AAAA records, no A/IPv4)
    ///
    /// 3 - Ipv4AndIpv6 (Query for A and AAAA records in parallel, uses whatever
    /// returns a successful response first)
    ///
    /// 4 - Ipv6thenIpv4 (Query for AAAA record, if that fails then query the A
    /// record)
    ///
    /// 5 - Ipv4thenIpv6 (Query for A record, if that fails then query the AAAA
    /// record)
    ///
    /// If you don't have IPv6 networking, then for better DNS performance it
    /// may be suitable to set this to Ipv4Only (1) as you will never ever use
    /// the AAAA record contents even if the AAAA record is successful instead
    /// of the A record.
    ///
    /// default: 5
    #[serde(default = "default_ip_lookup_strategy")]
    pub ip_lookup_strategy: u8,

    /// Max request size for file uploads in bytes. Defaults to 20MB.
    ///
    /// default: 20971520
    #[serde(default = "default_max_upload_size")]
    pub max_upload_size: u32,

    /// default: 192
    #[serde(default = "default_max_fetch_prev_events")]
    pub max_fetch_prev_events: u16,

    /// Default/base connection timeout. This is used only by URL
    /// previews and update/news endpoint checks.
    ///
    /// default: 10_000
    #[serde(default = "default_request_conn_timeout")]
    pub request_conn_timeout: u64,

    /// Default/base request timeout. The time waiting to receive more
    /// data from another server. This is used only by URL previews,
    /// update/news, and misc endpoint checks.
    ///
    /// default: 35_000
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// Default/base request total timeout. The time limit for a whole
    /// request. This is set very high to not cancel healthy requests while
    /// serving as a backstop. This is used only by URL previews and update/news
    /// endpoint checks.
    ///
    /// default: 320_000
    #[serde(default = "default_request_total_timeout")]
    pub request_total_timeout: u64,

    /// Default/base idle connection pool timeout. This is used only
    /// by URL previews and update/news endpoint checks.
    ///
    /// default: 5_000
    #[serde(default = "default_request_idle_timeout")]
    pub request_idle_timeout: u64,

    /// Default/base max idle connections per host. This is used only by URL
    /// previews and update/news endpoint checks. Defaults to 1 as generally the
    /// same open connection can be re-used.
    ///
    /// default: 1
    #[serde(default = "default_request_idle_per_host")]
    pub request_idle_per_host: u16,

    /// Appservice URL request connection timeout. Defaults to 35 seconds as
    /// generally appservices are hosted within the same network.
    ///
    /// default: 35
    #[serde(default = "default_appservice_timeout")]
    pub appservice_timeout: u64,

    /// Appservice URL idle connection pool timeout
    ///
    /// default: 300_000
    #[serde(default = "default_appservice_idle_timeout")]
    pub appservice_idle_timeout: u64,

    /// Notification gateway pusher idle connection pool timeout.
    ///
    /// default: 15_000
    #[serde(default = "default_pusher_idle_timeout")]
    pub pusher_idle_timeout: u64,

    /// Maximum time to receive a request from a client
    ///
    /// default: 75_000
    #[serde(default = "default_client_receive_timeout")]
    pub client_receive_timeout: u64,

    /// Maximum time to process a request received from a client
    ///
    /// default: 180_000
    #[serde(default = "default_client_request_timeout")]
    pub client_request_timeout: u64,

    /// Maximum time to transmit a response to a client
    ///
    /// default: 120_000
    #[serde(default = "default_client_response_timeout")]
    pub client_response_timeout: u64,

    /// Grace period for clean shutdown of client requests.
    ///
    /// default: 10_000
    #[serde(default = "default_client_shutdown_timeout")]
    pub client_shutdown_timeout: u64,

    /// Grace period for clean shutdown of federation requests.
    ///
    /// default: 5_000
    #[serde(default = "default_sender_shutdown_timeout")]
    pub sender_shutdown_timeout: u64,

    /// Path to a file on the system that gets read for additional registration
    /// tokens. Multiple tokens can be added if you separate them with
    /// whitespace
    ///
    /// palpo must be able to access the file, and it must not be empty
    ///
    /// example: "/etc/palpo/.reg_token"
    pub registration_token_file: Option<PathBuf>,

    /// Always calls /forget on behalf of the user if leaving a room. This is a
    /// part of MSC4267 "Automatically forgetting rooms on leave"
    #[serde(default)]
    pub forget_forced_upon_leave: bool,

    /// Set this to true to require authentication on the normally
    /// unauthenticated profile retrieval endpoints (GET)
    /// "/_matrix/client/v3/profile/{user_id}".
    ///
    /// This can prevent profile scraping.
    #[serde(default)]
    pub require_auth_for_profile_requests: bool,

    /// Enables registration. If set to false, no users can register on this
    /// server.
    ///
    /// If set to true without a token configured, users can register with no
    /// form of 2nd-step only if you set the following option to true:
    /// `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`
    ///
    /// If you would like registration only via token reg, please configure
    /// `registration_token` or `registration_token_file`.
    #[serde(default = "default_false")]
    pub allow_registration: bool,

    /// Enabling this setting opens registration to anyone without restrictions.
    /// This makes your server vulnerable to abuse
    #[serde(default)]
    pub yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse: bool,

    /// A static registration token that new users will have to provide when
    /// creating an account. If unset and `allow_registration` is true,
    /// you must set
    /// `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`
    /// to true to allow open registration without any conditions.
    ///
    /// YOU NEED TO EDIT THIS OR USE registration_token_file.
    ///
    /// example: "o&^uCtes4HPf0Vu@F20jQeeWE7"
    ///
    /// display: sensitive
    pub registration_token: Option<String>,

    /// Controls whether encrypted rooms and events are allowed.
    #[serde(default = "default_true")]
    pub allow_encryption: bool,

    /// Allow standard users to create rooms. Appservices and admins are always
    /// allowed to create rooms
    #[serde(default = "default_true")]
    pub allow_room_creation: bool,

    /// Set to false to disable users from joining or creating room versions
    /// that aren't officially supported by palpo.
    ///
    /// palpo officially supports room versions 6 - 11.
    ///
    /// palpo has slightly experimental (though works fine in practice)
    /// support for versions 3 - 5.
    #[serde(default = "default_true")]
    pub allow_unstable_room_versions: bool,

    /// Default room version palpo will create rooms with.
    ///
    /// Per spec, room version 11 is the default.
    ///
    /// default: 11
    #[serde(default = "default_default_room_version")]
    pub default_room_version: RoomVersionId,

    #[serde(default = "default_false")]
    pub allow_jaeger: bool,
    #[serde(default = "default_false")]
    pub tracing_flame: bool,

    #[serde(default = "default_true")]
    pub enable_admin_room: bool,

    pub appservice_registration_dir: Option<String>,

    /// Servers listed here will be used to gather public keys of other servers
    /// (notary trusted key servers).
    ///
    /// Currently, palpo doesn't support inbound batched key requests, so
    /// this list should only contain other Synapse servers.
    ///
    /// example: ["matrix.org", "tchncs.de"]
    ///
    /// default: ["matrix.org"]
    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,

    /// OpenID token expiration/TTL.
    ///
    /// These are the OpenID tokens that are primarily used for Matrix account
    /// integrations (e.g. Vector Integrations in Element), *not* OIDC/OpenID
    /// Connect/etc.
    ///
    /// default: 3600
    #[serde(default = "default_openid_token_ttl")]
    pub openid_token_ttl: u64,

    /// Allow an existing session to mint a login token for another client.
    /// This requires interactive authentication, but has security ramifications
    /// as a malicious client could use the mechanism to spawn more than one
    /// session.
    /// Enabled by default.
    #[serde(default = "default_true")]
    pub login_via_existing_session: bool,

    /// Login token expiration/TTL in milliseconds.
    ///
    /// These are short-lived tokens for the m.login.token endpoint.
    /// This is used to allow existing sessions to create new sessions.
    /// see login_via_existing_session.
    ///
    /// default: 120000
    #[serde(default = "default_login_token_ttl")]
    pub login_token_ttl: u64,

    #[serde(default = "default_refresh_token_ttl")]
    pub refresh_token_ttl: u64,

    #[serde(default = "default_session_ttl")]
    pub session_ttl: u64,

    /// List/vector of room IDs or room aliases that palpo will make newly
    /// registered users join. The rooms specified must be rooms that you have
    /// joined at least once on the server, and must be public.
    ///
    /// example: ["#palpo:example.com",
    /// "!eoIzvAvVwY23LPDay8:example.com"]
    ///
    /// default: []
    #[serde(default = "Vec::new")]
    pub auto_join_rooms: Vec<OwnedRoomOrAliasId>,

    /// Config option to automatically deactivate the account of any user who
    /// attempts to join a:
    /// - banned room
    /// - forbidden room alias
    /// - room alias or ID with a forbidden server name
    ///
    /// This may be useful if all your banned lists consist of toxic rooms or
    /// servers that no good faith user would ever attempt to join, and
    /// to automatically remediate the problem without any admin user
    /// intervention.
    ///
    /// This will also make the user leave all rooms. Federation (e.g. remote
    /// room invites) are ignored here.
    ///
    /// Defaults to false as rooms can be banned for non-moderation-related
    /// reasons and this performs a full user deactivation.
    #[serde(default)]
    pub auto_deactivate_banned_room_attempts: bool,

    /// Block non-admin local users from sending room invites (local and
    /// remote), and block non-admin users from receiving remote room invites.
    ///
    /// Admins are always allowed to send and receive all room invites.
    #[serde(default)]
    pub block_non_admin_invites: bool,

    /// Set this to true to allow your server's public room directory to be
    /// federated. Set this to false to protect against /publicRooms spiders,
    /// but will forbid external users from viewing your server's public room
    /// directory. If federation is disabled entirely (`allow_federation`), this
    /// is inherently false.
    #[serde(default)]
    pub allow_public_room_directory_over_federation: bool,

    /// Set this to true to allow your server's public room directory to be
    /// queried without client authentication (access token) through the Client
    /// APIs. Set this to false to protect against /publicRooms spiders.
    #[serde(default)]
    pub allow_public_room_directory_without_auth: bool,

    /// Set this to true to lock down your server's public room directory and
    /// only allow admins to publish rooms to the room directory. Unpublishing
    /// is still allowed by all users with this enabled.
    #[serde(default)]
    pub lockdown_public_room_directory: bool,

    /// This is a password that can be configured that will let you login to the
    /// server bot account (currently `@conduit`) for emergency troubleshooting
    /// purposes such as recovering/recreating your admin room, or inviting
    /// yourself back.
    ///
    /// See https://palpo.im/troubleshooting.html#lost-access-to-admin-room for other ways to get back into your admin room.
    ///
    /// Once this password is unset, all sessions will be logged out for
    /// security purposes.
    ///
    /// example: "x7k9m2p5#n8w1%q4r6"
    ///
    /// display: sensitive
    pub emergency_password: Option<String>,

    /// default: "/_matrix/push/v1/notify"
    #[serde(default = "default_notification_push_path")]
    pub notification_push_path: String,

    /// Set to true to allow user type "guest" registrations. Some clients like
    /// Element attempt to register guest users automatically.
    #[serde(default)]
    pub allow_guest_registration: bool,

    /// Set to true to log guest registrations in the admin room. Note that
    /// these may be noisy or unnecessary if you're a public homeserver.
    #[serde(default)]
    pub log_guest_registrations: bool,

    /// Set to true to allow guest registrations/users to auto join any rooms
    /// specified in `auto_join_rooms`.
    #[serde(default)]
    pub allow_guests_auto_join_rooms: bool,

    /// List of forbidden server names via regex patterns that we will block
    /// incoming AND outgoing federation with, and block client room joins /
    /// remote user invites.
    ///
    /// This check is applied on the room ID, room alias, sender server name,
    /// sender user's server name, inbound federation X-Matrix origin, and
    /// outbound federation handler.
    ///
    /// Basically "global" ACLs.
    ///
    /// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_server_names: RegexSet,

    /// List of forbidden server names via regex patterns that we will block all
    /// outgoing federated room directory requests for. Useful for preventing
    /// our users from wandering into bad servers or spaces.
    ///
    /// example: ["badserver\.tld$", "badphrase", "19dollarfortnitecards"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_room_directory_server_names: RegexSet,

    /// Vector list of IPv4 and IPv6 CIDR ranges / subnets *in quotes* that you
    /// do not want palpo to send outbound requests to. Defaults to
    /// RFC1918, unroutable, loopback, multicast, and testnet addresses for
    /// security.
    ///
    /// Please be aware that this is *not* a guarantee. You should be using a
    /// firewall with zones as doing this on the application layer may have
    /// bypasses.
    ///
    /// Currently this does not account for proxies in use like Synapse does.
    ///
    /// To disable, set this to be an empty vector (`[]`).
    ///
    /// Defaults to:
    /// ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12",
    /// "192.168.0.0/16", "100.64.0.0/10", "192.0.0.0/24", "169.254.0.0/16",
    /// "192.88.99.0/24", "198.18.0.0/15", "192.0.2.0/24", "198.51.100.0/24",
    /// "203.0.113.0/24", "224.0.0.0/4", "::1/128", "fe80::/10", "fc00::/7",
    /// "2001:db8::/32", "ff00::/8", "fec0::/10"]
    #[serde(default = "default_ip_range_denylist")]
    pub ip_range_denylist: Vec<String>,

    #[serde(default = "default_space_path")]
    pub space_path: String,

    // pub auto_acme: Option<AcmeConfig>,
    /// Whether to query the servers listed in trusted_servers first or query
    /// the origin server first. For best security, querying the origin server
    /// first is advised to minimize the exposure to a compromised trusted
    /// server. For maximum federation/join performance this can be set to true,
    /// however other options exist to query trusted servers first under
    /// specific high-load circumstances and should be evaluated before setting
    /// this to true.
    #[serde(default)]
    pub query_trusted_key_servers_first: bool,

    /// Whether to query the servers listed in trusted_servers first
    /// specifically on room joins. This option limits the exposure to a
    /// compromised trusted server to room joins only. The join operation
    /// requires gathering keys from many origin servers which can cause
    /// significant delays. Therefor this defaults to true to mitigate
    /// unexpected delays out-of-the-box. The security-paranoid or those
    /// willing to tolerate delays are advised to set this to false. Note that
    /// setting query_trusted_key_servers_first to true causes this option to
    /// be ignored.
    #[serde(default = "default_true")]
    pub query_trusted_key_servers_first_on_join: bool,

    /// Only query trusted servers for keys and never the origin server. This is
    /// intended for clusters or custom deployments using their trusted_servers
    /// as forwarding-agents to cache and deduplicate requests. Notary servers
    /// do not act as forwarding-agents by default, therefor do not enable this
    /// unless you know exactly what you are doing.
    #[serde(default)]
    pub only_query_trusted_key_servers: bool,

    /// Maximum number of keys to request in each trusted server batch query.
    ///
    /// default: 1024
    #[serde(default = "default_trusted_server_batch_size")]
    pub trusted_server_batch_size: usize,

    /// List of forbidden room aliases and room IDs as strings of regex
    /// patterns.
    ///
    /// Regex can be used or explicit contains matches can be done by just
    /// specifying the words (see example).
    ///
    /// This is checked upon room alias creation, custom room ID creation if
    /// used, and startup as warnings if any room aliases in your database have
    /// a forbidden room alias/ID.
    ///
    /// example: ["19dollarfortnitecards", "b[4a]droom", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_alias_names: RegexSet,

    /// List of forbidden username patterns/strings.
    ///
    /// Regex can be used or explicit contains matches can be done by just
    /// specifying the words (see example).
    ///
    /// This is checked upon username availability check, registration, and
    /// startup as warnings if any local users in your database have a forbidden
    /// username.
    ///
    /// example: ["administrator", "b[a4]dusernam[3e]", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_usernames: RegexSet,

    /// Retry failed and incomplete messages to remote servers immediately upon
    /// startup. This is called bursting. If this is disabled, said messages may
    /// not be delivered until more messages are queued for that server. Do not
    /// change this option unless server resources are extremely limited or the
    /// scale of the server's deployment is huge. Do not disable this unless you
    /// know what you are doing.
    #[serde(default = "default_true")]
    pub startup_netburst: bool,

    /// Messages are dropped and not reattempted. The `startup_netburst` option
    /// must be enabled for this value to have any effect. Do not change this
    /// value unless you know what you are doing. Set this value to -1 to
    /// reattempt every message without trimming the queues; this may consume
    /// significant disk. Set this value to 0 to drop all messages without any
    /// attempt at redelivery.
    ///
    /// default: 50
    #[serde(default = "default_startup_netburst_keep")]
    pub startup_netburst_keep: i64,

    // external structure; separate section
    #[serde(default)]
    pub logger: LoggerConfig,

    // external structure; separate section
    pub jwt: Option<JwtConfig>,

    // external structure; separate section
    pub proxy: Option<ProxyConfig>,

    // // external structure; separate section
    // pub ldap: Option<LdapConfig>,

    // external structure; separate section
    // display: hidden
    pub keypair: Option<KeypairConfig>,

    // external structure; separate section
    #[serde(default)]
    pub blurhash: BlurhashConfig,

    // external structure; separate section
    #[serde(default)]
    pub media: MediaConfig,

    // external structure; separate section
    pub turn: Option<TurnConfig>,

    // external structure; separate section
    #[serde(default)]
    pub url_preview: UrlPreviewConfig,

    // external structure; separate section
    #[serde(default)]
    pub admin: AdminConfig,

    // external structure; separate section
    #[serde(default)]
    pub presence: PresenceConfig,

    // external structure; separate section
    // display: hidden
    #[serde(default)]
    pub read_receipt: ReadReceiptConfig,

    // external structure; separate section
    #[serde(default)]
    pub typing: TypingConfig,

    // external structure; separate section
    #[serde(default)]
    pub compression: CompressionConfig,

    // external structure; separate section
    #[serde(default)]
    pub well_known: WellKnownConfig,

    // external structure; separate section
    #[serde(default)]
    pub federation: FederationConfig,

    // external structure; separate section
    #[serde(default)]
    pub http_client: HttpClientConfig,

    /// Enables configuration reload when the server receives SIGUSR1 on
    /// supporting platforms.
    ///
    /// default: true
    #[serde(default = "default_true")]
    pub config_reload_signal: bool,

    /// Toggles ignore checking/validating TLS certificates
    ///
    /// This applies to everything, including URL previews, federation requests,
    /// etc. This is a hidden argument that should NOT be used in production as
    /// it is highly insecure and I will personally yell at you if I catch you
    /// using this.
    #[serde(default)]
    pub allow_invalid_tls_certificates: bool,

    /// Number of sender task workers; determines sender parallelism. Default is
    /// '0' which means the value is determined internally, likely matching the
    /// number of tokio worker-threads or number of cores, etc. Override by
    /// setting a non-zero value.
    ///
    /// default: 0
    #[serde(default)]
    pub sender_workers: usize,

    // external structure; separate section
    pub oidc: Option<OidcConfig>,

    // // external structure; separate section
    // #[serde(default)]
    // pub appservice: BTreeMap<String, AppService>,
    #[serde(flatten)]
    #[allow(clippy::zero_sized_map_values)]
    // this is a catchall, the map shouldn't be zero at runtime
    catch_others: BTreeMap<String, IgnoredAny>,
}

impl ServerConfig {
    // pub fn enabled_ldap(&self) -> Option<&LdapConfig> {
    //     if let Some(ldap) = self.ldap.as_ref() {
    //         if ldap.enable { Some(ldap) } else { None }
    //     } else {
    //         None
    //     }
    // }

    pub fn enabled_jwt(&self) -> Option<&JwtConfig> {
        if let Some(jwt) = self.jwt.as_ref() {
            if jwt.enable { Some(jwt) } else { None }
        } else {
            None
        }
    }

    pub fn enabled_oidc(&self) -> Option<&OidcConfig> {
        if let Some(oidc) = self.oidc.as_ref() {
            if oidc.enable { Some(oidc) } else { None }
        } else {
            None
        }
    }

    pub fn enabled_turn(&self) -> Option<&TurnConfig> {
        if let Some(turn) = self.turn.as_ref() {
            if turn.enable { Some(turn) } else { None }
        } else {
            None
        }
    }

    pub fn enabled_federation(&self) -> Option<&FederationConfig> {
        if self.federation.enable {
            Some(&self.federation)
        } else {
            None
        }
    }

    pub fn well_known_client(&self) -> String {
        if let Some(url) = &self.well_known.client {
            url.to_string()
        } else {
            // If under proxy, you should set well-known client manually.
            if let Some(listenser) = self.listeners.first() {
                if listenser.enabled_tls().is_none() {
                    return format!("http://{}", self.server_name);
                }
            };
            format!("https://{}", self.server_name)
        }
    }

    pub fn well_known_server(&self) -> OwnedServerName {
        match &self.well_known.server {
            Some(server_name) => server_name.to_owned(),
            None => {
                if self.server_name.port().is_some() {
                    self.server_name.to_owned()
                } else {
                    format!("{}:443", self.server_name.host())
                        .try_into()
                        .expect("Host from valid hostname + :443 must be valid")
                }
            }
        }
    }

    pub fn check(&self) -> AppResult<()> {
        if cfg!(debug_assertions) {
            tracing::warn!("Note: palpo was built without optimisations (i.e. debug build)");
        }

        // if self
        //     .allow_invalid_tls_certificates_yes_i_know_what_the_fuck_i_am_doing_with_this_and_i_know_this_is_insecure
        // {
        //     tracing::warn!(
        //         "\n\nWARNING: \n\nTLS CERTIFICATE VALIDATION IS DISABLED, THIS IS HIGHLY INSECURE
        // AND SHOULD NOT BE USED IN PRODUCTION.\n\n"     );
        // }

        self.warn_deprecated();
        self.warn_unknown_key();

        // if self.sentry && self.sentry_endpoint.is_none() {
        //     return Err(AppError::internal(
        //         "sentry_endpoint",
        //         "Sentry cannot be enabled without an endpoint set"
        //     ));
        // }

        // if cfg!(all(
        //     feature = "hardened_malloc",
        //     feature = "jemalloc",
        //     not(target_env = "msvc")
        // )) {
        //     tracing::warn!(
        //         "hardened_malloc and jemalloc compile-time features are both enabled, this causes
        // \          jemalloc to be used."
        //     );
        // }

        // if cfg!(not(unix)) && self.unix_socket_path.is_some() {
        //     return Err(AppError::internal(
        //         "UNIX socket support is only available on *nix platforms. Please remove \
        //          'unix_socket_path' from your config.",
        //     ));
        // }

        // if self.unix_socket_path.is_none() && self.get_bind_hosts().is_empty() {
        //     return Err(AppError::internal("No TCP addresses were specified to listen on"));
        // }

        // if self.unix_socket_path.is_none() && self.get_bind_ports().is_empty() {
        //     return EErr(AppError::internal("No ports were specified to listen on"));
        // }

        // if self.unix_socket_path.is_none() {
        //     self.get_bind_addrs().iter().for_each(|addr| {
        //         use std::path::Path;

        //         if addr.ip().is_loopback() {
        //             tracing::info!(
        //                 "Found loopback listening address {addr}, running checks if we're in a \
        //                  container."
        //             );

        //             if Path::new("/proc/vz").exists() /* Guest */ &&
        // !Path::new("/proc/bz").exists()             /* Host */
        //             {
        //                 error!(
        //                     "You are detected using OpenVZ with a loopback/localhost listening \
        //                      address of {addr}. If you are using OpenVZ for containers and you
        // use \                      NAT-based networking to communicate with the host and
        // guest, this will \                      NOT work. Please change this to
        // \"0.0.0.0\". If this is expected, you \                      can ignore.",
        //                 );
        //             } else if Path::new("/.dockerenv").exists() {
        //                 error!(
        //                     "You are detected using Docker with a loopback/localhost listening \
        //                      address of {addr}. If you are using a reverse proxy on the host and
        // \                      require communication to palpo in the Docker container via
        // \                      NAT-based networking, this will NOT work. Please change
        // this to \                      \"0.0.0.0\". If this is expected, you can
        // ignore.",                 );
        //             } else if Path::new("/run/.containerenv").exists() {
        //                 error!(
        //                     "You are detected using Podman with a loopback/localhost listening \
        //                      address of {addr}. If you are using a reverse proxy on the host and
        // \                      require communication to palpo in the Podman container via
        // \                      NAT-based networking, this will NOT work. Please change
        // this to \                      \"0.0.0.0\". If this is expected, you can
        // ignore.",                 );
        //             }
        //         }
        //     });
        // }

        // yeah, unless the user built a debug build hopefully for local testing only
        if cfg!(not(debug_assertions)) && self.server_name == "your.server.name" {
            return Err(AppError::internal(
                "You must specify a valid server name for production usage of palpo.",
            ));
        }

        if self.emergency_password == Some(String::from("F670$2CP@Hw8mG7RY1$%!#Ic7YA")) {
            return Err(AppError::internal(
                "The public example emergency password is being used, this is insecure. Please \
                 change this.",
            ));
        }

        if self.emergency_password == Some(String::new()) {
            return Err(AppError::internal(
                "Emergency password was set to an empty string, this is not valid. Unset \
                 emergency_password to disable it or set it to a real password.",
            ));
        }

        // check if the user specified a registration token as `""`
        if self.registration_token == Some(String::new()) {
            return Err(AppError::internal(
                "Registration token was specified but is empty (\"\")",
            ));
        }

        // // check if we can read the token file path, and check if the file is empty
        // if self.registration_token_file.as_ref().is_some_and(|path| {
        //     let Ok(token) = std::fs::read_to_string(path).inspect_err(|e| {
        //         error!("Failed to read the registration token file: {e}");
        //     }) else {
        //         return true;
        //     };

        //     token == String::new()
        // }) {
        //     return Err(AppError::internal(
        //         "Registration token file was specified but is empty or failed to be read",
        //     ));
        // }

        if self.max_upload_size < 10_000_000 {
            tracing::warn!(
                "max request size is less than 100MB. Please increase it as this is too low for operable federation"
            );
        }

        // check if user specified valid IP CIDR ranges on startup
        for cidr in &self.ip_range_denylist {
            if let Err(_e) = ipaddress::IPAddress::parse(cidr) {
                return Err(AppError::internal(
                    "Parsing specified IP CIDR range from string failed: {e}.",
                ));
            }
        }

        //     if self.allow_registration
        //         &&
        // !self.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
        //         && self.registration_token.is_none()
        //         && self.registration_token_file.is_none()
        //     {
        //         return Err(AppError::internal(
        //             "!! You have `allow_registration` enabled without a token configured in your
        // config \              which means you are allowing ANYONE to register on your
        // palpo instance without \              any 2nd-step (e.g. registration token). If
        // this is not the intended behaviour, \              please set a registration
        // token. For security and safety reasons, palpo will \              shut down. If
        // you are extra sure this is the desired behaviour you want, please \              
        // set the following config option to true:
        // `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse`",
        //         ));
        //     }

        // if self.allow_registration
        //     && self.yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse
        //     && self.registration_token.is_none()
        //     && self.registration_token_file.is_none()
        // {
        //     warn!(
        //         "Open registration is enabled via setting \
        //          `yes_i_am_very_very_sure_i_want_an_open_registration_server_prone_to_abuse` and
        // \          `allow_registration` to true without a registration token configured.
        // You are \          expected to be aware of the risks now. If this is not the
        // desired behaviour, \          please set a registration token."
        //     );
        // }

        if self.presence.allow_outgoing && !self.presence.allow_local {
            return Err(AppError::internal(
                "Outgoing presence requires allowing local presence. Please enable \
                 'allow_local_presence' or disable outgoing presence.",
            ));
        }

        self.url_preview.check();

        // if let Some(Either::Right(_)) = self.url_preview_bound_interface.as_ref() {
        //     if !matches!(OS, "android" | "fuchsia" | "linux") {
        //         return Err(AppError::internal(
        //             "url_preview_bound_interface",
        //             "Not a valid IP address. Interface names not supported on {OS}."
        //         ));
        //     }
        // }

        // if !Server::available_room_versions().any(|(version, _)| version ==
        // self.default_room_version) {     return Err(AppError::internal(formmat!(
        //         "Room version {:?} is not available",
        //         self.default_room_version
        //     )));
        // }

        Ok(())
    }
    /// Iterates over all the keys in the config file and warns if there is a
    /// deprecated key specified
    fn warn_deprecated(&self) {
        debug!("Checking for deprecated config keys");
        let mut was_deprecated = false;
        for key in self
            .catch_others
            .keys()
            .filter(|key| DEPRECATED_KEYS.iter().any(|s| s == key))
        {
            warn!("Config parameter \"{}\" is deprecated, ignoring.", key);
            was_deprecated = true;
        }

        if was_deprecated {
            warn!(
                "Read palpo config documentation at https://palpo.im/guide/configuration.html and check your \
                 configuration if any new configuration parameters should be adjusted"
            );
        }
    }

    /// iterates over all the catchall keys (unknown config options) and warns
    /// if there are any.
    fn warn_unknown_key(&self) {
        debug!("Checking for unknown config keys");
        for key in self.catch_others.keys().filter(
            |key| "config".to_owned().ne(key.to_owned()), // "config" is expected
        ) {
            warn!(
                "Config parameter \"{}\" is unknown to palpo, ignoring.",
                key
            );
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AllowedOrigins(Vec<String>);

impl AllowedOrigins {
    pub fn from_env() -> anyhow::Result<Self> {
        let allowed_origins = required_var("WEB_ALLOWED_ORIGINS")?
            .split(',')
            .map(ToString::to_string)
            .collect();

        Ok(Self(allowed_origins))
    }

    pub fn contains(&self, value: &HeaderValue) -> bool {
        self.0.iter().any(|it| it == value)
    }
}

#[config_example(filename = "palpo-example.toml", section = "tls")]
#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    #[serde(default = "default_true")]
    pub enable: bool,

    /// Path to a valid TLS certificate file.
    ///
    /// example: "/path/to/my/certificate.crt"
    pub cert: String,

    /// Path to a valid TLS certificate private key.
    ///
    /// example: "/path/to/my/certificate.key"
    pub key: String,

    /// Whether to listen and allow for HTTP and HTTPS connections (insecure!)
    #[serde(default)]
    pub dual_protocol: bool,
}

fn default_listener() -> Vec<ListenerConfig> {
    vec![Default::default()]
}
fn default_listen_address() -> String {
    "0.0.0.0:8008".into()
}
fn default_server_name() -> OwnedServerName {
    OwnedServerName::try_from("change.palpo.im").expect("default server name should be valid")
}

fn default_db_cache_capacity_mb() -> f64 {
    300.0
}

fn default_palpo_cache_capacity_modifier() -> f64 {
    1.0
}

fn default_pdu_cache_capacity() -> u32 {
    150_000
}

fn default_trusted_server_batch_size() -> usize {
    256
}

fn default_space_path() -> String {
    "./space".into()
}

fn default_startup_netburst_keep() -> i64 {
    50
}
fn default_login_token_ttl() -> u64 {
    2 * 60_000
}
fn default_refresh_token_ttl() -> u64 {
    2 * 60_000
}
fn default_session_ttl() -> u64 {
    60 * 60_000
}
fn default_openid_token_ttl() -> u64 {
    60 * 60_000
}

fn default_ip_lookup_strategy() -> u8 {
    5
}

fn default_cleanup_interval() -> u32 {
    60_000 // every minute
}
fn default_request_timeout() -> u64 {
    35_000
}

fn default_max_upload_size() -> u32 {
    100 * 1024 * 1024 // Default to 20 MB
}

fn default_max_concurrent_requests() -> u16 {
    100
}

fn default_max_fetch_prev_events() -> u16 {
    100_u16
}

fn default_auth_chain_cache_capacity() -> u32 {
    parallelism_scaled_u32(10_000).saturating_add(100_000)
}
fn default_roomid_space_hierarchy_cache_capacity() -> u32 {
    parallelism_scaled_u32(1000)
}

fn default_trusted_servers() -> Vec<OwnedServerName> {
    vec![OwnedServerName::try_from("matrix.org").unwrap()]
}

fn default_rust_log() -> String {
    "warn".to_owned()
}

fn default_log_format() -> String {
    "json".to_owned()
}

fn default_default_room_version() -> RoomVersionId {
    RoomVersionId::V11
}

fn default_ip_range_denylist() -> Vec<String> {
    vec![
        "127.0.0.0/8".to_owned(),
        "10.0.0.0/8".to_owned(),
        "172.16.0.0/12".to_owned(),
        "192.168.0.0/16".to_owned(),
        "100.64.0.0/10".to_owned(),
        "192.0.0.0/24".to_owned(),
        "169.254.0.0/16".to_owned(),
        "192.88.99.0/24".to_owned(),
        "198.18.0.0/15".to_owned(),
        "192.0.2.0/24".to_owned(),
        "198.51.100.0/24".to_owned(),
        "203.0.113.0/24".to_owned(),
        "224.0.0.0/4".to_owned(),
        "::1/128".to_owned(),
        "fe80::/10".to_owned(),
        "fc00::/7".to_owned(),
        "2001:db8::/32".to_owned(),
        "ff00::/8".to_owned(),
        "fec0::/10".to_owned(),
    ]
}

fn parallelism_scaled_u32(val: u32) -> u32 {
    let val = val.try_into().expect("failed to cast u32 to usize");
    parallelism_scaled(val).try_into().unwrap_or(u32::MAX)
}
fn parallelism_scaled(val: usize) -> usize {
    val.saturating_mul(sys::available_parallelism())
}

fn default_server_name_event_data_cache_capacity() -> u32 {
    parallelism_scaled_u32(100_000).saturating_add(500_000)
}
fn default_new_user_displayname_suffix() -> String {
    "💕".to_owned()
}

fn default_request_total_timeout() -> u64 {
    320_000
}

fn default_request_conn_timeout() -> u64 {
    10_000
}

fn default_request_idle_timeout() -> u64 {
    5_000
}

fn default_request_idle_per_host() -> u16 {
    1_000
}
fn default_appservice_timeout() -> u64 {
    35_000
}

fn default_appservice_idle_timeout() -> u64 {
    300_000
}

fn default_client_receive_timeout() -> u64 {
    75_000
}

fn default_client_request_timeout() -> u64 {
    180_000
}

fn default_client_response_timeout() -> u64 {
    120_000
}

fn default_client_shutdown_timeout() -> u64 {
    15_000
}

fn default_sender_shutdown_timeout() -> u64 {
    5_000
}

fn default_notification_push_path() -> String {
    "/_matrix/push/v1/notify".to_owned()
}

fn default_pusher_idle_timeout() -> u64 {
    15_000
}
