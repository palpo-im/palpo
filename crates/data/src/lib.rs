use std::sync::OnceLock;

use diesel::{Connection, PgConnection};
use diesel_async::RunQueryDsl;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use url::Url;

extern crate tracing;
#[macro_use]
mod macros;
mod config;
pub use palpo_core as core;

pub use crate::config::DbConfig;

pub mod full_text_search;

pub mod pool;
pub use pool::{DieselPool, PgPooledConnection, PoolError};

pub mod appservice;
pub mod media;
pub mod misc;
pub mod room;
pub mod schema;
pub mod sending;
pub mod user;

mod error;
pub use error::DataError;

use crate::core::Seqnum;

pub type DataResult<T> = Result<T, DataError>;

pub static DIESEL_POOL: OnceLock<DieselPool> = OnceLock::new();
pub static REPLICA_POOL: OnceLock<Option<DieselPool>> = OnceLock::new();

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub fn init(config: &DbConfig) {
    let pool = DieselPool::new(&config.url, config).expect("diesel pool should be created");
    DIESEL_POOL.set(pool).expect("diesel pool should be set");
    migrate(config);
}

/// Run pending migrations using a one-off synchronous connection.
///
/// `diesel_migrations` only operates on synchronous connections, so this
/// establishes a dedicated `PgConnection` separate from the async pool. It also
/// doubles as a fail-fast connectivity check at startup.
pub fn migrate(config: &DbConfig) {
    let url = connection_url(config, &config.url);
    let mut conn = PgConnection::establish(&url).expect("db connect should worked");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("migrate db should worked");
}

pub async fn connect() -> Result<PgPooledConnection, PoolError> {
    match DIESEL_POOL.get().expect("diesel pool should set").get().await {
        Ok(conn) => Ok(conn),
        Err(e) => {
            tracing::error!("db connect error: {e}");
            Err(e)
        }
    }
}
pub fn status() -> deadpool::managed::Status {
    DIESEL_POOL.get().expect("diesel pool should set").status()
}

pub fn connection_url(config: &DbConfig, url: &str) -> String {
    let mut url = Url::parse(url).expect("Invalid database URL");

    if config.enforce_tls {
        maybe_append_url_param(&mut url, "sslmode", "require");
    }

    // Configure the time it takes for diesel to return an error when there is full packet loss
    // between the application and the database.
    maybe_append_url_param(
        &mut url,
        "tcp_user_timeout",
        &config.tcp_timeout.to_string(),
    );

    url.into()
}

fn maybe_append_url_param(url: &mut Url, key: &str, value: &str) {
    if !url.query_pairs().any(|(k, _)| k == key) {
        url.query_pairs_mut().append_pair(key, value);
    }
}

pub async fn next_sn() -> DataResult<Seqnum> {
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT nextval('occur_sn_seq')")
        .get_result::<Seqnum>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
pub async fn curr_sn() -> DataResult<Seqnum> {
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT last_value from occur_sn_seq")
        .get_result::<Seqnum>(&mut connect().await?)
        .await
        .map_err(Into::into)
}
