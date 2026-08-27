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

pub fn init(config: &DbConfig) {
    let (query_pool_size, coordination_pool_size) = split_pool_size(config.pool_size);

    // Migrations use a one-off synchronous connection. Run them before creating the
    // long-lived pools so startup does not temporarily exceed the configured budget.
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

fn split_pool_size(total: u32) -> (usize, usize) {
    let coordination = coordination_pool_capacity(total);
    (total as usize - coordination, coordination)
}

/// Number of connections reserved for database-backed coordination inside the
/// configured total pool budget.
pub fn coordination_pool_capacity(total: u32) -> usize {
    assert!(
        total >= 2,
        "db.pool_size must be at least 2 so database-backed coordination cannot deadlock query work"
    );
    let total = total as usize;
    (total / 2).clamp(1, 8)
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

#[cfg(test)]
mod migration_tests {
    use std::fs;
    use std::path::Path;

    use diesel::migration::MigrationSource;
    use diesel::pg::Pg;
    use diesel_migrations::EmbeddedMigrations;

    use super::{MIGRATIONS, coordination_pool_capacity, split_pool_size};

    #[test]
    fn coordination_pool_stays_inside_the_configured_budget() {
        for total in 2..=64 {
            let (queries, coordination) = split_pool_size(total);
            assert!(queries > 0);
            assert!(coordination > 0);
            assert_eq!(queries + coordination, total as usize);
            assert!(coordination <= 8);
            assert_eq!(coordination, coordination_pool_capacity(total));
        }
    }

    #[test]
    fn coordination_capacity_matches_the_pool_split_policy() {
        assert_eq!(split_pool_size(2), (1, 1));
        assert_eq!(split_pool_size(3), (2, 1));
        assert_eq!(split_pool_size(4), (2, 2));
        assert_eq!(split_pool_size(5), (3, 2));
        assert_eq!(split_pool_size(20), (12, 8));
    }

    #[test]
    #[should_panic(expected = "db.pool_size must be at least 2")]
    fn a_single_connection_cannot_support_database_coordination() {
        split_pool_size(1);
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
