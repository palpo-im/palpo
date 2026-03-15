use std::sync::{Arc, OnceLock};
use std::time::Duration;

use diesel::prelude::*;
use diesel::r2d2::{self, State};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use scheduled_thread_pool::ScheduledThreadPool;
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
    // Check if already initialized (fast path)
    if DIESEL_POOL.get().is_some() {
        return; // Already initialized, skip
    }
    
    let builder = r2d2::Pool::builder()
        .max_size(config.pool_size)
        .min_idle(config.min_idle)
        .connection_timeout(Duration::from_millis(config.connection_timeout))
        .connection_customizer(Box::new(config::ConnectionConfig {
            statement_timeout: config.statement_timeout,
        }))
        .thread_pool(Arc::new(ScheduledThreadPool::new(config.helper_threads)));

    let pool =
        DieselPool::new(&config.url, config, builder).expect("diesel pool should be created");
    
    // Use get_or_init to safely handle concurrent initialization
    // This ensures only one pool is created even with multiple threads
    let set_result = DIESEL_POOL.set(pool);
    if set_result.is_err() {
        // Pool was already set by another thread, that's okay
        tracing::debug!("Database pool already initialized by another thread");
    } else {
        // We initialized the pool, run migrations
        migrate();
    }
}
pub fn migrate() {
    let conn = &mut connect().expect("db connect should worked");
    conn.run_pending_migrations(MIGRATIONS)
        .expect("migrate db should worked");
}

pub fn connect() -> Result<PgPooledConnection, PoolError> {
    match DIESEL_POOL.get().expect("diesel pool should set").get() {
        Ok(conn) => Ok(conn),
        Err(e) => {
            tracing::error!("db connect error: {e}");
            Err(e)
        }
    }
}
pub fn state() -> State {
    DIESEL_POOL.get().expect("diesel pool should set").state()
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

pub fn next_sn() -> DataResult<Seqnum> {
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT nextval('occur_sn_seq')")
        .get_result::<Seqnum>(&mut connect()?)
        .map_err(Into::into)
}
pub fn curr_sn() -> DataResult<Seqnum> {
    diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT last_value from occur_sn_seq")
        .get_result::<Seqnum>(&mut connect()?)
        .map_err(Into::into)
}
