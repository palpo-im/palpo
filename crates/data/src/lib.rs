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
pub static COORDINATION_POOL: OnceLock<DieselPool> = OnceLock::new();
pub static REPLICA_POOL: OnceLock<Option<DieselPool>> = OnceLock::new();

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

/// Largest automatically derived coordination pool.
///
/// Coordination connections sit idle inside a transaction while the work they fence runs
/// on the query pool, so a deployment rarely benefits from more of them than this. An
/// operator who needs a higher ceiling sets `db.coordination_pool_size` explicitly.
const MAX_DERIVED_COORDINATION_POOL_SIZE: usize = 16;

pub fn init(config: &DbConfig) {
    assert!(
        config.pool_size >= 2,
        "db.pool_size must be at least 2 so database-backed coordination cannot starve query work"
    );
    let (query_pool_size, coordination_pool_size) = split_pool_size(config);

    // Migrations run on a one-off synchronous connection, and the pools below only open
    // connections on demand. Migrating first still keeps startup single-connection, which
    // matters when `db.pool_size` is already sized against `max_connections`.
    migrate(config);

    let pool = DieselPool::new(&config.url, config, query_pool_size, "queries")
        .expect("diesel query pool should be created");
    DIESEL_POOL.set(pool).expect("diesel pool should be set");
    let coordination_pool =
        DieselPool::new(&config.url, config, coordination_pool_size, "coordination")
            .expect("diesel coordination pool should be created");
    COORDINATION_POOL
        .set(coordination_pool)
        .expect("diesel coordination pool should be set");
}

fn split_pool_size(config: &DbConfig) -> (usize, usize) {
    let coordination = coordination_pool_capacity(config.pool_size, config.coordination_pool_size);
    (
        (config.pool_size as usize).max(2) - coordination,
        coordination,
    )
}

/// Number of connections reserved for database-backed coordination inside the
/// configured total pool budget.
///
/// `configured` is the operator's explicit `db.coordination_pool_size`. When it is unset
/// the capacity is derived from the total budget; either way the result is at least 1 and
/// always leaves at least one connection for ordinary queries, so this never panics on a
/// hostile configuration. `ServerConfig::check` rejects the invalid combinations up front
/// with a message an operator can act on.
pub fn coordination_pool_capacity(total: u32, configured: Option<u32>) -> usize {
    // A single connection cannot serve both roles; `init` and the server config check
    // both refuse to start in that case, so only keep this arm self-consistent.
    let total = (total as usize).max(2);
    let derived = (total / 4).clamp(1, MAX_DERIVED_COORDINATION_POOL_SIZE);
    let wanted = configured.map_or(derived, |configured| configured as usize);
    wanted.clamp(1, total - 1)
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
    match DIESEL_POOL
        .get()
        .expect("diesel pool should set")
        .get()
        .await
    {
        Ok(conn) => Ok(conn),
        Err(e) => {
            tracing::error!("db connect error: {e}");
            Err(e)
        }
    }
}

/// Get a connection reserved for operations which must hold a database lock while
/// ordinary queries continue on the primary pool.
///
/// Both pools are carved out of `db.pool_size`; using this pool never increases the
/// configured steady-state connection budget.
pub async fn coordination_connect() -> Result<PgPooledConnection, PoolError> {
    match COORDINATION_POOL
        .get()
        .expect("diesel coordination pool should set")
        .get()
        .await
    {
        Ok(conn) => Ok(conn),
        Err(e) => {
            tracing::error!("db coordination connect error: {e}");
            Err(e)
        }
    }
}
/// Status of the query pool. The coordination pool is reported separately by
/// [`coordination_status`].
pub fn status() -> deadpool::managed::Status {
    DIESEL_POOL.get().expect("diesel pool should set").status()
}

/// Status of the pool backing [`coordination_connect`].
pub fn coordination_status() -> deadpool::managed::Status {
    COORDINATION_POOL
        .get()
        .expect("diesel coordination pool should set")
        .status()
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

#[cfg(test)]
mod migration_tests {
    use std::fs;
    use std::path::Path;

    use diesel::migration::MigrationSource;
    use diesel::pg::Pg;
    use diesel_migrations::EmbeddedMigrations;

    use super::{
        DbConfig, MAX_DERIVED_COORDINATION_POOL_SIZE, MIGRATIONS, coordination_pool_capacity,
        split_pool_size,
    };

    fn db_config(pool_size: u32, coordination_pool_size: Option<u32>) -> DbConfig {
        DbConfig {
            url: String::new(),
            pool_size,
            coordination_pool_size,
            tcp_timeout: 0,
            connection_timeout: 0,
            statement_timeout: 0,
            enforce_tls: false,
        }
    }

    #[test]
    fn coordination_pool_stays_inside_the_configured_budget() {
        for total in 2..=256 {
            for configured in [None, Some(0), Some(1), Some(4), Some(total), Some(u32::MAX)] {
                let (queries, coordination) = split_pool_size(&db_config(total, configured));
                assert!(queries > 0, "total={total} configured={configured:?}");
                assert!(coordination > 0, "total={total} configured={configured:?}");
                assert_eq!(queries + coordination, total as usize);
                assert_eq!(
                    coordination,
                    coordination_pool_capacity(total, configured),
                    "total={total} configured={configured:?}"
                );
            }
        }
    }

    #[test]
    fn derived_coordination_capacity_leaves_most_connections_for_queries() {
        assert_eq!(split_pool_size(&db_config(2, None)), (1, 1));
        assert_eq!(split_pool_size(&db_config(4, None)), (3, 1));
        // The historical default budget keeps 8 of its 10 connections for queries.
        assert_eq!(split_pool_size(&db_config(10, None)), (8, 2));
        assert_eq!(split_pool_size(&db_config(20, None)), (15, 5));
        assert_eq!(split_pool_size(&db_config(64, None)), (48, 16));
    }

    #[test]
    fn derived_coordination_capacity_is_capped() {
        assert_eq!(
            coordination_pool_capacity(1_000, None),
            MAX_DERIVED_COORDINATION_POOL_SIZE
        );
    }

    #[test]
    fn an_explicit_coordination_capacity_overrides_the_derived_one() {
        assert_eq!(split_pool_size(&db_config(20, Some(2))), (18, 2));
        // An explicit value may exceed the derived cap.
        assert_eq!(split_pool_size(&db_config(64, Some(32))), (32, 32));
    }

    #[test]
    fn an_out_of_range_coordination_capacity_still_leaves_a_usable_split() {
        // `ServerConfig::check` rejects these before startup; the split must stay
        // self-consistent for any other caller.
        assert_eq!(split_pool_size(&db_config(10, Some(0))), (9, 1));
        assert_eq!(split_pool_size(&db_config(10, Some(10))), (1, 9));
        assert_eq!(split_pool_size(&db_config(10, Some(u32::MAX))), (1, 9));
    }

    fn validate_index_guard(statement: &str) -> Result<Option<bool>, &'static str> {
        let tokens = statement
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_uppercase)
            .collect::<Vec<_>>();
        let Some(operation) = tokens.first().map(String::as_str) else {
            return Ok(None);
        };

        let mut cursor = 1;
        if operation == "CREATE" && tokens.get(cursor).map(String::as_str) == Some("UNIQUE") {
            cursor += 1;
        }
        if !matches!(operation, "CREATE" | "DROP")
            || tokens.get(cursor).map(String::as_str) != Some("INDEX")
        {
            return Ok(None);
        }

        cursor += 1;
        let concurrently = tokens.get(cursor).map(String::as_str) == Some("CONCURRENTLY");
        if concurrently {
            cursor += 1;
        }

        let guard = tokens
            .get(cursor..)
            .unwrap_or_default()
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>();
        let starts_like_guard = guard
            .first()
            .is_some_and(|token| matches!(*token, "IF" | "NOT" | "EXISTS"));

        match operation {
            "CREATE" if concurrently || starts_like_guard => {
                if guard.starts_with(&["IF", "NOT", "EXISTS"]) {
                    Ok(Some(concurrently))
                } else {
                    Err("CREATE INDEX guard must use IF NOT EXISTS")
                }
            }
            "DROP" if concurrently || starts_like_guard => {
                if guard.starts_with(&["IF", "EXISTS"]) {
                    Ok(Some(concurrently))
                } else {
                    Err("DROP INDEX guard must use IF EXISTS")
                }
            }
            _ => Ok(Some(concurrently)),
        }
    }

    #[test]
    fn index_migrations_guard_existence_and_transaction_mode_correctly() {
        let migrations = <EmbeddedMigrations as MigrationSource<Pg>>::migrations(&MIGRATIONS)
            .expect("embedded migrations should be readable");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

        let mut checked = 0usize;
        for entry in fs::read_dir(&dir).expect("migrations directory should be readable") {
            let entry = entry.expect("migration directory entry should be readable");
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let migration = migrations
                .iter()
                .find(|migration| migration.name().to_string() == name)
                .unwrap_or_else(|| panic!("{name} should be embedded"));

            for file in ["up.sql", "down.sql"] {
                let Ok(sql) = fs::read_to_string(entry.path().join(file)) else {
                    continue;
                };
                // Migration files document the query sites they index, and those comments
                // mention the very keywords checked below.
                let sql = sql
                    .lines()
                    .map(|line| line.split("--").next().unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut has_concurrent_index = false;
                for statement in sql
                    .split(';')
                    .filter(|statement| !statement.trim().is_empty())
                {
                    match validate_index_guard(statement) {
                        Ok(Some(concurrently)) => has_concurrent_index |= concurrently,
                        Ok(None) => {}
                        Err(error) => panic!("{name}/{file}: {error}: {statement}"),
                    }
                }

                if has_concurrent_index {
                    assert!(
                        !migration.metadata().run_in_transaction(),
                        "{name}: CREATE/DROP INDEX CONCURRENTLY cannot run inside a transaction"
                    );
                    assert_eq!(
                        sql.matches(';').count(),
                        1,
                        "{name}/{file}: concurrent migration batches must contain one statement"
                    );
                }
                checked += 1;
            }
        }

        assert!(checked > 0, "no migration SQL was checked");
    }

    #[test]
    fn concurrent_index_guards_are_recognized_structurally() {
        for statement in [
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS example_idx ON example (id)",
            "CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS example_idx ON example (id)",
            "CREATE UNIQUE INDEX IF NOT EXISTS example_idx ON example (id)",
            "DROP INDEX CONCURRENTLY IF EXISTS example_idx",
            "DROP INDEX IF EXISTS example_idx",
        ] {
            assert!(validate_index_guard(statement).is_ok());
        }

        for statement in [
            "CREATE INDEX CONCURRENTLY example_idx ON example (id)",
            "CREATE INDEX CONCURRENTLY IF EXISTS example_idx ON example (id)",
            "CREATE UNIQUE INDEX CONCURRENTLY IF EXISTS example_idx ON example (id)",
            "CREATE UNIQUE INDEX IF EXISTS example_idx ON example (id)",
            "DROP INDEX CONCURRENTLY example_idx",
            "DROP INDEX CONCURRENTLY IF NOT EXISTS example_idx",
            "DROP INDEX IF NOT EXISTS example_idx",
        ] {
            assert!(
                validate_index_guard(statement).is_err(),
                "invalid guard was accepted: {statement}"
            );
        }
    }
}
