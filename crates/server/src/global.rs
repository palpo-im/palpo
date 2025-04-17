use std::collections::{BTreeMap, HashMap};
use std::error::Error as StdError;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use std::{future, iter};

use diesel::prelude::*;
use futures_util::FutureExt;
use hickory_resolver::Resolver as HickoryResolver;
use hickory_resolver::config::*;
use hickory_resolver::name_server::TokioConnectionProvider;
use hyper_util::client::legacy::connect::dns::{GaiResolver, Name as HyperName};
use ipaddress::IPAddress;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use salvo::oapi::ToSchema;
use serde::Serialize;
use tokio::sync::{Semaphore, broadcast, watch::Receiver};
use tower_service::Service as TowerService;

use crate::core::UnixMillis;
use crate::core::client::sync_events;
use crate::core::federation::discovery::{OldVerifyKey, ServerSigningKeys};
use crate::core::identifiers::*;
use crate::core::serde::{Base64, CanonicalJsonObject, JsonValue, RawJsonValue};
use crate::core::signatures::Ed25519KeyPair;
use crate::data::connect;
use crate::data::misc::DbServerSigningKeys;
use crate::data::schema::*;
use crate::{AppResult, MatrixError, ServerConfig, SigningKeys};

pub const MXC_LENGTH: usize = 32;
pub const DEVICE_ID_LENGTH: usize = 10;
pub const TOKEN_LENGTH: usize = 32;
pub const SESSION_ID_LENGTH: usize = 32;
pub const AUTO_GEN_PASSWORD_LENGTH: usize = 15;
pub const RANDOM_USER_ID_LENGTH: usize = 10;

type SyncHandle = (
    Option<String>,                                                  // since
    Receiver<Option<AppResult<sync_events::v3::SyncEventsResBody>>>, // rx
);
type TlsNameMap = HashMap<String, (Vec<IpAddr>, u16)>;
type RateLimitState = (Instant, u32); // Time if last failed try, number of failed tries
// type SyncHandle = (
//     Option<String>,                                         // since
//     Receiver<Option<AppResult<sync_events::v3::Response>>>, // rx
// );

pub type LazyRwLock<T> = LazyLock<RwLock<T>>;
pub static TLS_NAME_OVERRIDE: LazyRwLock<TlsNameMap> = LazyLock::new(Default::default);
pub static STABLE_ROOM_VERSIONS: LazyLock<Vec<RoomVersionId>> = LazyLock::new(|| {
    vec![
        RoomVersionId::V6,
        RoomVersionId::V7,
        RoomVersionId::V8,
        RoomVersionId::V9,
        RoomVersionId::V10,
        RoomVersionId::V11,
    ]
});
pub static UNSTABLE_ROOM_VERSIONS: LazyLock<Vec<RoomVersionId>> = LazyLock::new(|| {
    vec![
        RoomVersionId::V2,
        RoomVersionId::V3,
        RoomVersionId::V4,
        RoomVersionId::V5,
    ]
});
pub static BAD_EVENT_RATE_LIMITER: LazyRwLock<HashMap<OwnedEventId, RateLimitState>> = LazyLock::new(Default::default);
pub static BAD_SIGNATURE_RATE_LIMITER: LazyRwLock<HashMap<Vec<String>, RateLimitState>> =
    LazyLock::new(Default::default);
pub static BAD_QUERY_RATE_LIMITER: LazyRwLock<HashMap<OwnedServerName, RateLimitState>> =
    LazyLock::new(Default::default);
pub static SERVER_NAME_RATE_LIMITER: LazyRwLock<HashMap<OwnedServerName, Arc<Semaphore>>> =
    LazyLock::new(Default::default);
pub static ROOM_ID_FEDERATION_HANDLE_TIME: LazyRwLock<HashMap<OwnedRoomId, (OwnedEventId, Instant)>> =
    LazyLock::new(Default::default);
pub static STATERES_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(Default::default);
pub static APPSERVICE_IN_ROOM_CACHE: LazyRwLock<HashMap<OwnedRoomId, HashMap<String, bool>>> =
    LazyRwLock::new(Default::default);
pub static ROTATE: LazyLock<RotationHandler> = LazyLock::new(Default::default);
pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Handles "rotation" of long-polling requests. "Rotation" in this context is similar to "rotation" of log files and the like.
///
/// This is utilized to have sync workers return early and release read locks on the database.
pub struct RotationHandler(broadcast::Sender<()>, broadcast::Receiver<()>);

#[derive(Serialize, ToSchema, Clone, Copy, Debug)]
pub struct EmptyObject {}

impl RotationHandler {
    pub fn new() -> Self {
        let (s, r) = broadcast::channel(1);
        Self(s, r)
    }

    pub fn watch(&self) -> impl Future<Output = ()> {
        let mut r = self.0.subscribe();

        async move {
            let _ = r.recv().await;
        }
    }

    pub fn fire(&self) {
        let _ = self.0.send(());
    }
}

impl Default for RotationHandler {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Resolver {
    inner: GaiResolver,
    overrides: Arc<RwLock<TlsNameMap>>,
}

impl Resolver {
    pub fn new(overrides: Arc<RwLock<TlsNameMap>>) -> Self {
        Resolver {
            inner: GaiResolver::new(),
            overrides,
        }
    }
}

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        self.overrides
            .read()
            .unwrap()
            .get(name.as_str())
            .and_then(|(override_name, port)| {
                override_name.first().map(|first_name| {
                    let x: Box<dyn Iterator<Item = SocketAddr> + Send> =
                        Box::new(iter::once(SocketAddr::new(*first_name, *port)));
                    let x: Resolving = Box::pin(future::ready(Ok(x)));
                    x
                })
            })
            .unwrap_or_else(|| {
                let this = &mut self.inner.clone();
                Box::pin(
                    TowerService::<HyperName>::call(
                        this,
                        // Beautiful hack, please remove this in the future.
                        HyperName::from_str(name.as_str()).expect("reqwest Name is just wrapper for hyper-util Name"),
                    )
                    .map(|result| {
                        result
                            .map(|addrs| -> Addrs { Box::new(addrs) })
                            .map_err(|err| -> Box<dyn StdError + Send + Sync> { Box::new(err) })
                    }),
                )
            })
    }
}

pub fn config() -> &'static crate::config::ServerConfig {
    crate::config::get()
}

pub fn server_user() -> String {
    format!("@palpo:{}", crate::server_name())
}

/// Returns this server's keypair.
pub fn keypair() -> &'static Ed25519KeyPair {
    static KEYPAIR: OnceLock<Ed25519KeyPair> = OnceLock::new();
    KEYPAIR.get_or_init(|| {
        if let Some(keypair) = &crate::config().keypair {
            let bytes = base64::decode(&keypair.document).expect("server keypair is invalid base64 string");
            Ed25519KeyPair::from_der(&bytes, keypair.version.clone()).expect("invalid server Ed25519KeyPair")
        } else {
            crate::utils::generate_keypair()
        }
    })
}

pub fn well_known_client() -> String {
    let config = crate::config();
    if let Some(url) = &config.well_known.client {
        url.to_string()
    } else {
        format!("https://{}", config.server_name)
    }
}

pub fn well_known_server() -> OwnedServerName {
    let config = crate::config();
    match &config.well_known.server {
        Some(server_name) => server_name.to_owned(),
        None => {
            if config.server_name.port().is_some() {
                config.server_name.to_owned()
            } else {
                format!("{}:443", config.server_name.host())
                    .try_into()
                    .expect("Host from valid hostname + :443 must be valid")
            }
        }
    }
}

pub fn cidr_range_denylist() -> &'static [IPAddress] {
    static CIDR_RANGE_DENYLIST: OnceLock<Vec<IPAddress>> = OnceLock::new();
    CIDR_RANGE_DENYLIST.get_or_init(|| {
        let conf = crate::config();
        conf.ip_range_denylist
            .iter()
            .map(IPAddress::parse)
            .inspect(|cidr| trace!("Denied CIDR range: {cidr:?}"))
            .collect::<Result<_, String>>()
            .expect("Invalid CIDR range in config")
    })
}
/// Returns a reqwest client which can be used to send requests
pub fn default_client() -> reqwest::Client {
    // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
    static DEFAULT_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    DEFAULT_CLIENT
        .get_or_init(|| {
            reqwest_client_builder(crate::config())
                .expect("failed to build request clinet")
                .build()
                .expect("failed to build request clinet")
        })
        .clone()
}

/// Returns a client used for resolving .well-knowns
pub fn federation_client() -> reqwest::Client {
    static FEDERATION_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    FEDERATION_CLIENT
        .get_or_init(|| {
            let conf = config();
            // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
            let tls_name_override = Arc::new(RwLock::new(TlsNameMap::new()));

            // let jwt_decoding_key = conf
            //     .jwt_secret
            //     .as_ref()
            //     .map(|secret| jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()));

            reqwest_client_builder(conf)
                .expect("build reqwest client failed")
                .dns_resolver(Arc::new(Resolver::new(tls_name_override.clone())))
                .timeout(Duration::from_secs(2 * 60))
                .build()
                .expect("build reqwest client failed")
        })
        .clone()
}

pub fn valid_cidr_range(ip: &IPAddress) -> bool {
    cidr_range_denylist().iter().all(|cidr| !cidr.includes(ip))
}

pub fn server_name() -> &'static ServerName {
    config().server_name.as_ref()
}
pub fn listen_addr() -> &'static str {
    config().listen_addr.deref()
}

pub fn max_request_size() -> u32 {
    config().max_request_size
}

pub fn max_fetch_prev_events() -> u16 {
    config().max_fetch_prev_events
}

pub fn allow_registration() -> bool {
    config().allow_registration
}

pub fn allow_encryption() -> bool {
    config().allow_encryption
}

pub fn allow_federation() -> bool {
    config().allow_federation
}

pub fn allow_room_creation() -> bool {
    config().allow_room_creation
}

pub fn allow_unstable_room_versions() -> bool {
    config().allow_unstable_room_versions
}

pub fn default_room_version() -> RoomVersionId {
    config().room_version.clone()
}

pub fn enable_lightning_bolt() -> bool {
    config().enable_lightning_bolt
}

pub fn allow_check_for_updates() -> bool {
    config().allow_check_for_updates
}

pub fn trusted_servers() -> &'static [OwnedServerName] {
    &config().trusted_servers
}

pub fn dns_resolver() -> &'static HickoryResolver<TokioConnectionProvider> {
    static DNS_RESOLVER: OnceLock<HickoryResolver<TokioConnectionProvider>> = OnceLock::new();
    DNS_RESOLVER.get_or_init(|| {
        HickoryResolver::builder_with_config(ResolverConfig::default(), TokioConnectionProvider::default()).build()
    })
}

pub fn jwt_decoding_key() -> Option<&'static jsonwebtoken::DecodingKey> {
    static JWT_DECODING_KEY: OnceLock<Option<jsonwebtoken::DecodingKey>> = OnceLock::new();
    JWT_DECODING_KEY
        .get_or_init(|| {
            config()
                .jwt_secret
                .as_ref()
                .map(|secret| jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()))
        })
        .as_ref()
}

pub fn turn_password() -> &'static str {
    &config().turn_password
}

pub fn turn_ttl() -> u64 {
    config().turn_ttl
}

pub fn turn_uris() -> &'static [String] {
    &config().turn_uris
}

pub fn turn_username() -> &'static str {
    &config().turn_username
}

pub fn turn_secret() -> &'static String {
    &config().turn_secret
}

pub fn emergency_password() -> Option<&'static str> {
    config().emergency_password.as_deref()
}

pub fn allow_local_presence() -> bool {
    config().allow_local_presence
}

pub fn allow_incoming_presence() -> bool {
    config().allow_incoming_presence
}

pub fn allow_outcoming_presence() -> bool {
    config().allow_outgoing_presence
}

pub fn presence_idle_timeout_s() -> u64 {
    config().presence_idle_timeout_s
}

pub fn presence_offline_timeout_s() -> u64 {
    config().presence_offline_timeout_s
}

pub fn supported_room_versions() -> Vec<RoomVersionId> {
    let mut room_versions: Vec<RoomVersionId> = vec![];
    room_versions.extend(STABLE_ROOM_VERSIONS.clone());
    if config().allow_unstable_room_versions {
        room_versions.extend(UNSTABLE_ROOM_VERSIONS.clone());
    };
    room_versions
}

pub fn supports_room_version(room_version: &RoomVersionId) -> bool {
    supported_room_versions().contains(room_version)
}

pub fn add_signing_key_from_trusted_server(
    from_server: &ServerName,
    new_keys: ServerSigningKeys,
) -> AppResult<SigningKeys> {
    let key_data = server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(from_server))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;

    let prev_keys: Option<ServerSigningKeys> = key_data.map(|key_data| serde_json::from_value(key_data)).transpose()?;

    if let Some(mut prev_keys) = prev_keys {
        let ServerSigningKeys {
            verify_keys,
            old_verify_keys,
            ..
        } = new_keys;

        prev_keys.verify_keys.extend(verify_keys);
        prev_keys.old_verify_keys.extend(old_verify_keys);
        prev_keys.valid_until_ts = new_keys.valid_until_ts;

        diesel::insert_into(server_signing_keys::table)
            .values(DbServerSigningKeys {
                server_id: from_server.to_owned(),
                key_data: serde_json::to_value(&prev_keys)?,
                updated_at: UnixMillis::now(),
                created_at: UnixMillis::now(),
            })
            .on_conflict(server_signing_keys::server_id)
            .do_update()
            .set((
                server_signing_keys::key_data.eq(serde_json::to_value(&prev_keys)?),
                server_signing_keys::updated_at.eq(UnixMillis::now()),
            ))
            .execute(&mut connect()?)?;
        Ok(prev_keys.into())
    } else {
        diesel::insert_into(server_signing_keys::table)
            .values(DbServerSigningKeys {
                server_id: from_server.to_owned(),
                key_data: serde_json::to_value(&new_keys)?,
                updated_at: UnixMillis::now(),
                created_at: UnixMillis::now(),
            })
            .on_conflict(server_signing_keys::server_id)
            .do_update()
            .set((
                server_signing_keys::key_data.eq(serde_json::to_value(&new_keys)?),
                server_signing_keys::updated_at.eq(UnixMillis::now()),
            ))
            .execute(&mut connect()?)?;
        Ok(new_keys.into())
    }
}
pub fn add_signing_key_from_origin(origin: &ServerName, new_keys: ServerSigningKeys) -> AppResult<SigningKeys> {
    let key_data = server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(origin))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;

    let prev_keys: Option<ServerSigningKeys> = key_data.map(|key_data| serde_json::from_value(key_data)).transpose()?;

    if let Some(mut prev_keys) = prev_keys {
        let ServerSigningKeys {
            verify_keys,
            old_verify_keys,
            ..
        } = new_keys;

        // Moving `verify_keys` no longer present to `old_verify_keys`
        for (key_id, key) in prev_keys.verify_keys {
            if !verify_keys.contains_key(&key_id) {
                prev_keys
                    .old_verify_keys
                    .insert(key_id, OldVerifyKey::new(prev_keys.valid_until_ts, key.key));
            }
        }

        prev_keys.verify_keys = verify_keys;
        prev_keys.old_verify_keys.extend(old_verify_keys);
        prev_keys.valid_until_ts = new_keys.valid_until_ts;

        diesel::insert_into(server_signing_keys::table)
            .values(DbServerSigningKeys {
                server_id: origin.to_owned(),
                key_data: serde_json::to_value(&prev_keys)?,
                updated_at: UnixMillis::now(),
                created_at: UnixMillis::now(),
            })
            .on_conflict(server_signing_keys::server_id)
            .do_update()
            .set((
                server_signing_keys::key_data.eq(serde_json::to_value(&prev_keys)?),
                server_signing_keys::updated_at.eq(UnixMillis::now()),
            ))
            .execute(&mut connect()?)?;
        Ok(prev_keys.into())
    } else {
        diesel::insert_into(server_signing_keys::table)
            .values(DbServerSigningKeys {
                server_id: origin.to_owned(),
                key_data: serde_json::to_value(&new_keys)?,
                updated_at: UnixMillis::now(),
                created_at: UnixMillis::now(),
            })
            .on_conflict(server_signing_keys::server_id)
            .do_update()
            .set((
                server_signing_keys::key_data.eq(serde_json::to_value(&new_keys)?),
                server_signing_keys::updated_at.eq(UnixMillis::now()),
            ))
            .execute(&mut connect()?)?;
        Ok(new_keys.into())
    }
}

// /// This returns an empty `Ok(None)` when there are no keys found for the server.
// pub fn signing_keys_for(origin: &ServerName) -> AppResult<Option<SigningKeys>> {
//     let key_data = server_signing_keys::table
//         .filter(server_signing_keys::server_id.eq(origin))
//         .select(server_signing_keys::key_data)
//         .first::<JsonValue>(&mut connect()?)
//         .optional()?;
//     if let Some(key_data) = key_data {
//         Ok(serde_json::from_value(key_data).map(Option::Some)?)
//     } else {
//         Ok(None)
//     }
// }

/// Filters the key map of multiple servers down to keys that should be accepted given the expiry time,
/// room version, and timestamp of the paramters
pub fn filter_keys_server_map(
    keys: BTreeMap<String, SigningKeys>,
    timestamp: UnixMillis,
    room_version_id: &RoomVersionId,
) -> BTreeMap<String, BTreeMap<String, Base64>> {
    keys.into_iter()
        .filter_map(|(server, keys)| {
            filter_keys_single_server(keys, timestamp, room_version_id).map(|keys| (server, keys))
        })
        .collect()
}

/// Filters the keys of a single server down to keys that should be accepted given the expiry time,
/// room version, and timestamp of the paramters
pub fn filter_keys_single_server(
    keys: SigningKeys,
    timestamp: UnixMillis,
    room_version_id: &RoomVersionId,
) -> Option<BTreeMap<String, Base64>> {
    if keys.valid_until_ts > timestamp
        // valid_until_ts MUST be ignored in room versions 1, 2, 3, and 4.
        // https://spec.matrix.org/v1.10/server-server-api/#get_matrixkeyv2server
        || matches!(room_version_id, RoomVersionId::V1
                    | RoomVersionId::V2
                    | RoomVersionId::V4
                    | RoomVersionId::V3)
    {
        // Given that either the room version allows stale keys, or the valid_until_ts is
        // in the future, all verify_keys are valid
        let mut map: BTreeMap<_, _> = keys.verify_keys.into_iter().map(|(id, key)| (id, key.key)).collect();

        map.extend(keys.old_verify_keys.into_iter().filter_map(|(id, key)| {
            // Even on old room versions, we don't allow old keys if they are expired
            if key.expired_ts > timestamp {
                Some((id, key.key))
            } else {
                None
            }
        }));

        Some(map)
    } else {
        None
    }
}

pub fn space_path() -> &'static str {
    config().space_path.deref()
}

pub fn media_path(server_name: &ServerName, media_id: &str) -> PathBuf {
    let server_name = if server_name == crate::server_name().as_str() {
        "_"
    } else {
        server_name.as_str()
    };
    let mut r = PathBuf::new();
    r.push(space_path());
    r.push("media");
    r.push(server_name);
    // let extension = extension.unwrap_or_default();
    // if !extension.is_empty() {
    //     r.push(format!("{media_id}.{extension}"));
    // } else {
    r.push(media_id);
    // }
    r
}

pub fn shutdown() {
    SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
    // On shutdown
    info!(target: "shutdown-sync", "Received shutdown notification, notifying sync helpers...");
    ROTATE.fire();
}

fn reqwest_client_builder(config: &ServerConfig) -> AppResult<reqwest::ClientBuilder> {
    let reqwest_client_builder = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 3));

    // TODO: add proxy support
    // if let Some(proxy) = config.to_proxy()? {
    //     reqwest_client_builder = reqwest_client_builder.proxy(proxy);
    // }

    Ok(reqwest_client_builder)
}

pub fn parse_incoming_pdu(pdu: &RawJsonValue) -> AppResult<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
    let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
        tracing::warn!("Error parsing incoming event {:?}: {:?}", pdu, e);
        MatrixError::bad_json("Invalid PDU in server response")
    })?;

    let room_id: OwnedRoomId = value
        .get("room_id")
        .and_then(|id| RoomId::parse(id.as_str()?).ok())
        .ok_or(MatrixError::invalid_param("Invalid room id in pdu"))?;

    let room_version_id = crate::room::state::get_room_version(&room_id)?;

    let (event_id, value) = match crate::event::gen_event_id_canonical_json(pdu, &room_version_id) {
        Ok(t) => t,
        Err(_) => {
            // Event could not be converted to canonical json
            return Err(MatrixError::invalid_param("Could not convert event to canonical json.").into());
        }
    };
    Ok((event_id, value, room_id))
}
