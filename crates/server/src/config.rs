use std::iter::once;
use std::ops::Deref;
use std::path::Path;
use std::sync::{LazyLock, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use figment::Figment;
use figment::providers::{Env, Format, Json, Toml, Yaml};
use ipaddress::IPAddress;

mod server;
pub use server::*;
mod admin;
pub use admin::*;
// mod appservice;
// pub use appservice::*;
mod jwt;
pub use jwt::*;
mod blurhash;
pub use blurhash::*;
// mod cache;
// pub use cache::*;
mod compression;
pub use compression::*;
mod db;
pub use db::*;
// mod dns;
// pub use dns::*;
mod federation;
pub use federation::*;
mod http_client;
pub use http_client::*;
// mod ldap;
// pub use ldap::*;
mod logger;
pub use logger::*;
mod media;
pub use media::*;
mod presence;
pub use presence::*;
mod proxy;
pub use proxy::*;
mod read_receipt;
pub use read_receipt::*;
mod turn;
pub use turn::*;
mod typing;
pub use typing::*;
mod url_preview;
pub use url_preview::*;
mod well_known;
pub use well_known::*;
mod oidc;
pub use oidc::*;

use crate::AppResult;
use crate::core::client::discovery::capabilities::RoomVersionStability;
use crate::core::identifiers::*;
use crate::core::signatures::Ed25519KeyPair;

pub static CONFIG: OnceLock<ServerConfig> = OnceLock::new();

pub static STABLE_ROOM_VERSIONS: LazyLock<Vec<RoomVersionId>> = LazyLock::new(|| {
    vec![
        RoomVersionId::V6,
        RoomVersionId::V7,
        RoomVersionId::V8,
        RoomVersionId::V9,
        RoomVersionId::V10,
        RoomVersionId::V11,
        RoomVersionId::V12,
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

fn figment_from_path<P: AsRef<Path>>(path: P) -> Figment {
    let ext = path
        .as_ref()
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    match ext {
        "yaml" | "yml" => Figment::new().merge(Yaml::file(path)),
        "json" => Figment::new().merge(Json::file(path)),
        "toml" => Figment::new().merge(Toml::file(path)),
        _ => panic!("unsupported config file format: {ext}"),
    }
}

pub fn init(config_path: impl AsRef<Path>) {
    let config_path = config_path.as_ref();
    if !config_path.exists() {
        panic!("config file not found: `{}`", config_path.display());
    }

    let raw_conf = figment_from_path(config_path).merge(Env::prefixed("PALPO_").global());
    let conf = match raw_conf.extract::<ServerConfig>() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("it looks like your config is invalid. The following error occurred: {e}");
            std::process::exit(1);
        }
    };

    CONFIG.set(conf).expect("config should be set once");
}
pub fn reload(_path: impl AsRef<Path>) -> AppResult<()> {
    // TODO: reload config
    Ok(())
}

pub fn get() -> &'static ServerConfig {
    CONFIG.get().unwrap()
}

pub static SERVER_USER_ID: OnceLock<OwnedUserId> = OnceLock::new();
pub fn server_user_id() -> &'static UserId {
    SERVER_USER_ID.get_or_init(|| {
        format!("@palpo:{}", get().server_name)
            .try_into()
            .expect("invalid server user ID")
    })
}

pub fn server_user() -> crate::data::user::DbUser {
    crate::data::user::get_user(server_user_id()).expect("server user should exist in the database")
}

pub fn space_path() -> &'static str {
    get().space_path.deref()
}
pub fn server_name() -> &'static ServerName {
    get().server_name.deref()
}

static ADMIN_ALIAS: OnceLock<OwnedRoomAliasId> = OnceLock::new();
pub fn admin_alias() -> &'static RoomAliasId {
    ADMIN_ALIAS.get_or_init(|| {
        let alias = format!("#admins:{}", get().server_name);
        alias
            .try_into()
            .expect("admin alias should be a valid room alias")
    })
}

pub fn appservice_registration_dir() -> Option<&'static str> {
    get().appservice_registration_dir.as_deref()
}

/// Returns this server's keypair.
pub fn keypair() -> &'static Ed25519KeyPair {
    static KEYPAIR: OnceLock<Ed25519KeyPair> = OnceLock::new();
    KEYPAIR.get_or_init(|| {
        if let Some(keypair) = &get().keypair {
            let bytes = STANDARD
                .decode(&keypair.document)
                .expect("server keypair is invalid base64 string");
            Ed25519KeyPair::from_der(&bytes, keypair.version.clone())
                .expect("invalid server Ed25519KeyPair")
        } else {
            crate::utils::generate_keypair()
        }
    })
}

pub fn valid_cidr_range(ip: &IPAddress) -> bool {
    cidr_range_denylist().iter().all(|cidr| !cidr.includes(ip))
}

pub fn cidr_range_denylist() -> &'static [IPAddress] {
    static CIDR_RANGE_DENYLIST: OnceLock<Vec<IPAddress>> = OnceLock::new();
    CIDR_RANGE_DENYLIST.get_or_init(|| {
        let conf = get();
        conf.ip_range_denylist
            .iter()
            .map(IPAddress::parse)
            .inspect(|cidr| trace!("Denied CIDR range: {cidr:?}"))
            .collect::<Result<_, String>>()
            .expect("invalid CIDR range in config")
    })
}

pub fn jwt_decoding_key() -> Option<&'static jsonwebtoken::DecodingKey> {
    static JWT_DECODING_KEY: OnceLock<Option<jsonwebtoken::DecodingKey>> = OnceLock::new();
    JWT_DECODING_KEY
        .get_or_init(|| {
            get()
                .jwt
                .as_ref()
                .map(|jwt| jsonwebtoken::DecodingKey::from_secret(jwt.secret.as_bytes()))
        })
        .as_ref()
}

pub fn supported_room_versions() -> Vec<RoomVersionId> {
    let mut room_versions: Vec<RoomVersionId> = vec![];
    room_versions.extend(STABLE_ROOM_VERSIONS.clone());
    if get().allow_unstable_room_versions {
        room_versions.extend(UNSTABLE_ROOM_VERSIONS.clone());
    };
    room_versions
}

pub fn supports_room_version(room_version: &RoomVersionId) -> bool {
    supported_room_versions().contains(room_version)
}

pub type RoomVersion = (RoomVersionId, RoomVersionStability);
pub fn available_room_versions() -> impl Iterator<Item = RoomVersion> {
    let unstable_room_versions = UNSTABLE_ROOM_VERSIONS
        .iter()
        .cloned()
        .zip(once(RoomVersionStability::Unstable).cycle());

    STABLE_ROOM_VERSIONS
        .iter()
        .cloned()
        .zip(once(RoomVersionStability::Stable).cycle())
        .chain(unstable_room_versions)
}

#[inline]
fn supported_stability(stability: &RoomVersionStability) -> bool {
    get().allow_unstable_room_versions || *stability == RoomVersionStability::Stable
}
