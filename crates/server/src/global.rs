use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::net::IpAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock, OnceLock, RwLock};
use std::time::Instant;

use diesel::prelude::*;
use hickory_resolver::Resolver as HickoryResolver;
use hickory_resolver::config::*;
use hickory_resolver::name_server::TokioConnectionProvider;
use salvo::oapi::ToSchema;
use serde::Serialize;
use tokio::sync::{Semaphore, broadcast};

use crate::core::appservice::Registration;
use crate::core::federation::discovery::{OldVerifyKey, ServerSigningKeys};
use crate::core::identifiers::*;
use crate::core::serde::{Base64, JsonValue};
use crate::core::{Seqnum, UnixMillis};
use crate::data::misc::DbServerSigningKeys;
use crate::data::schema::*;
use crate::data::user::{NewDbUser, NewDbUserDevice};
use crate::appservice::DbRegistration;
use crate::data::{connect, diesel_exists};
use crate::utils::{MutexMap, MutexMapGuard, SeqnumQueue, SeqnumQueueFuture, SeqnumQueueGuard};
use crate::{AppResult, SigningKeys};

pub const MXC_LENGTH: usize = 32;
pub const DEVICE_ID_LENGTH: usize = 10;
pub const TOKEN_LENGTH: usize = 32;
pub const SESSION_ID_LENGTH: usize = 32;
pub const AUTO_GEN_PASSWORD_LENGTH: usize = 15;
pub const RANDOM_USER_ID_LENGTH: usize = 10;

pub type TlsNameMap = HashMap<String, (Vec<IpAddr>, u16)>;
type RateLimitState = (Instant, u32); // Time if last failed try, number of failed tries

pub type RoomMutexMap = MutexMap<OwnedRoomId, ()>;
pub type RoomMutexGuard = MutexMapGuard<OwnedRoomId, ()>;

pub type LazyRwLock<T> = LazyLock<RwLock<T>>;
pub static TLS_NAME_OVERRIDE: LazyRwLock<TlsNameMap> = LazyLock::new(Default::default);
pub static BAD_EVENT_RATE_LIMITER: LazyRwLock<HashMap<OwnedEventId, RateLimitState>> =
    LazyLock::new(Default::default);
pub static BAD_SIGNATURE_RATE_LIMITER: LazyRwLock<HashMap<Vec<String>, RateLimitState>> =
    LazyLock::new(Default::default);
pub static BAD_QUERY_RATE_LIMITER: LazyRwLock<HashMap<OwnedServerName, RateLimitState>> =
    LazyLock::new(Default::default);
pub static SERVER_NAME_RATE_LIMITER: LazyRwLock<HashMap<OwnedServerName, Arc<Semaphore>>> =
    LazyLock::new(Default::default);
pub static ROOM_ID_FEDERATION_HANDLE_TIME: LazyRwLock<
    HashMap<OwnedRoomId, (OwnedEventId, Instant)>,
> = LazyLock::new(Default::default);
pub static APPSERVICE_IN_ROOM_CACHE: LazyRwLock<HashMap<OwnedRoomId, HashMap<String, bool>>> =
    LazyRwLock::new(Default::default);
pub static ROTATE: LazyLock<RotationHandler> = LazyLock::new(Default::default);
pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
pub static SEQNUM_QUEUE: LazyLock<SeqnumQueue> = LazyLock::new(Default::default);

/// Handles "rotation" of long-polling requests. "Rotation" in this context is similar to "rotation"
/// of log files and the like.
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

pub fn queue_seqnum(sn: Seqnum) -> SeqnumQueueGuard {
    SEQNUM_QUEUE.push(sn)
}

pub fn seqnum_reach(sn: Seqnum) -> SeqnumQueueFuture {
    SEQNUM_QUEUE.reach(sn)
}

pub fn dns_resolver() -> &'static HickoryResolver<TokioConnectionProvider> {
    static DNS_RESOLVER: OnceLock<HickoryResolver<TokioConnectionProvider>> = OnceLock::new();
    DNS_RESOLVER.get_or_init(|| {
        HickoryResolver::builder_with_config(
            ResolverConfig::default(),
            TokioConnectionProvider::default(),
        )
        .build()
    })
}

pub fn appservices() -> &'static Vec<Registration> {
    use figment::Figment;
    use figment::providers::{Format, Toml, Yaml};
    static APPSERVICES: OnceLock<Vec<Registration>> = OnceLock::new();

    APPSERVICES.get_or_init(|| {
        let mut appservices = vec![];
        let Some(registration_dir) = crate::config::appservice_registration_dir() else {
            return appservices;
        };
        tracing::info!("Appservice registration dir: {}", registration_dir);
        let mut exist_ids = HashSet::new();
        let Ok(entries) = std::fs::read_dir(registration_dir) else {
            return appservices;
        };

        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();

            if path.is_dir() {
                continue;
            }
            let Some(ext) = path.extension() else {
                continue;
            };
            let registration = match ext.to_str() {
                Some("yaml") | Some("yml") => match Figment::new()
                    .merge(Yaml::file(&path))
                    .extract::<Registration>()
                {
                    Ok(registration) => registration,
                    Err(e) => {
                        tracing::error!(
                            "It looks like your config `{}` is invalid. Error occurred: {e}",
                            path.display()
                        );
                        continue;
                    }
                },
                Some("toml") => match Figment::new()
                    .merge(Toml::file(&path))
                    .extract::<Registration>()
                {
                    Ok(registration) => registration,
                    Err(e) => {
                        tracing::error!(
                            "It looks like your config `{}` is invalid. Error occurred: {e}",
                            path.display()
                        );
                        continue;
                    }
                },
                _ => {
                    continue;
                }
            };

            if exist_ids.contains(&registration.id) {
                tracing::error!("Duplicate appservice registration id: {}", registration.id);
                continue;
            }
            exist_ids.insert(registration.id.clone());
            appservices.push(registration.clone());

            // Ensure the appservice registration is in the database so that
            // event forwarding (via appservice::all()) can find it.
            let db_registration: DbRegistration = registration.clone().into();
            let mut conn = connect().expect("db connect failed");
            if let Err(e) = diesel::insert_into(appservice_registrations::table)
                .values(&db_registration)
                .on_conflict(appservice_registrations::id)
                .do_update()
                .set((
                    appservice_registrations::url.eq(&db_registration.url),
                    appservice_registrations::as_token.eq(&db_registration.as_token),
                    appservice_registrations::hs_token.eq(&db_registration.hs_token),
                    appservice_registrations::sender_localpart.eq(&db_registration.sender_localpart),
                    appservice_registrations::namespaces.eq(&db_registration.namespaces),
                    appservice_registrations::rate_limited.eq(&db_registration.rate_limited),
                    appservice_registrations::protocols.eq(&db_registration.protocols),
                    appservice_registrations::receive_ephemeral.eq(&db_registration.receive_ephemeral),
                    appservice_registrations::device_management.eq(&db_registration.device_management),
                ))
                .execute(&mut conn)
            {
                tracing::error!("Failed to save appservice registration to database: {e}");
            }
            drop(conn);

            let user_id = OwnedUserId::try_from(format!(
                "@{}:{}",
                registration.sender_localpart,
                crate::config::get().server_name
            ))
            .unwrap();
            let mut conn = connect().expect("db connect failed");
            if !diesel_exists!(users::table.filter(users::id.eq(&user_id)), &mut conn)
                .expect("db query failed")
            {
                diesel::insert_into(users::table)
                    .values(NewDbUser {
                        id: user_id.to_owned(),
                        ty: None,
                        is_admin: false,
                        is_guest: false,
                        is_local: true,
                        localpart: user_id.localpart().to_owned(),
                        server_name: user_id.server_name().to_owned(),
                        appservice_id: Some(registration.id.clone()),
                        created_at: UnixMillis::now(),
                    })
                    .execute(&mut conn)
                    .expect("db query failed");
            }

            if !diesel_exists!(
                user_devices::table.filter(user_devices::user_id.eq(&user_id)),
                &mut conn
            )
            .expect("db query failed")
            {
                diesel::insert_into(user_devices::table)
                    .values(NewDbUserDevice {
                        user_id,
                        device_id: OwnedDeviceId::from("_"),
                        display_name: Some("[Default]".to_string()),
                        user_agent: None,
                        is_hidden: true,
                        last_seen_ip: None,
                        last_seen_at: None,
                        created_at: UnixMillis::now(),
                    })
                    .execute(&mut conn)
                    .expect("db query failed");
            }
        }
        appservices
    })
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

    let prev_keys: Option<ServerSigningKeys> = key_data.map(serde_json::from_value).transpose()?;

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
pub fn add_signing_key_from_origin(
    origin: &ServerName,
    new_keys: ServerSigningKeys,
) -> AppResult<SigningKeys> {
    let key_data = server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(origin))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect()?)
        .optional()?;

    let prev_keys: Option<ServerSigningKeys> = key_data.map(serde_json::from_value).transpose()?;

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

/// Filters the key map of multiple servers down to keys that should be accepted given the expiry
/// time, room version, and timestamp of the paramters
pub fn filter_keys_server_map(
    keys: BTreeMap<String, SigningKeys>,
    timestamp: UnixMillis,
    room_version: &RoomVersionId,
) -> BTreeMap<String, BTreeMap<String, Base64>> {
    keys.into_iter()
        .filter_map(|(server, keys)| {
            filter_keys_single_server(keys, timestamp, room_version).map(|keys| (server, keys))
        })
        .collect()
}

/// Filters the keys of a single server down to keys that should be accepted given the expiry time,
/// room version, and timestamp of the paramters
pub fn filter_keys_single_server(
    keys: SigningKeys,
    timestamp: UnixMillis,
    room_version: &RoomVersionId,
) -> Option<BTreeMap<String, Base64>> {
    if keys.valid_until_ts > timestamp
        // valid_until_ts MUST be ignored in room versions 1, 2, 3, and 4.
        // https://spec.matrix.org/v1.10/server-server-api/#get_matrixkeyv2server
        || matches!(room_version, RoomVersionId::V1
                    | RoomVersionId::V2
                    | RoomVersionId::V4
                    | RoomVersionId::V3)
    {
        // Given that either the room version allows stale keys, or the valid_until_ts is
        // in the future, all verify_keys are valid
        let mut map: BTreeMap<_, _> = keys
            .verify_keys
            .into_iter()
            .map(|(id, key)| (id, key.key))
            .collect();

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

pub fn shutdown() {
    SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
    // On shutdown
    info!(target: "shutdown-sync", "received shutdown notification, notifying sync helpers...");
    ROTATE.fire();
}

pub fn get_servers_from_users(users: &[OwnedUserId]) -> Vec<OwnedServerName> {
    let mut servers = HashSet::new();
    for user in users {
        let server_name = user.server_name().to_owned();
        servers.insert(server_name);
    }
    servers.into_iter().collect()
}

pub fn palpo_user(server_name: &ServerName) -> OwnedUserId {
    UserId::parse_with_server_name("palpo", server_name).expect("@palpo:server_name is valid")
}
