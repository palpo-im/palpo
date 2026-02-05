//! Configuration for setting up database pools
//!
//! - `DATABASE_URL`: The URL of the postgres database to use.
//! - `READ_ONLY_REPLICA_URL`: The URL of an optional postgres read-only replica database.
//! - `DB_DIESEL_POOL_SIZE`: The number of connections of the primary database.
//! - `DB_REPLICA_POOL_SIZE`: The number of connections of the read-only / replica database.
//! - `DB_PRIMARY_MIN_IDLE`: The primary pool will maintain at least this number of connections.
//! - `DB_REPLICA_MIN_IDLE`: The replica pool will maintain at least this number of connections.
//! - `DB_OFFLINE`: If set to `leader` then use the read-only follower as if it was the leader. If
//!   set to `follower` then act as if `READ_ONLY_REPLICA_URL` was unset.
//! - `READ_ONLY_MODE`: If defined (even as empty) then force all connections to be read-only.
//! - `DB_TCP_TIMEOUT_MS`: TCP timeout in milliseconds. See the doc comment for more details.

use std::fmt;

use diesel::prelude::*;
use diesel::r2d2::{self, CustomizeConnection};
use serde::{Deserialize, Serialize};

use crate::core::serde::default_false;

fn default_db_pool_size() -> u32 {
    10
}
fn default_tcp_timeout() -> u64 {
    10000
}
fn default_connection_timeout() -> u64 {
    30000
}
fn default_statement_timeout() -> u64 {
    30000
}
fn default_helper_threads() -> usize {
    10
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct DbConfig {
    /// Settings for the primary database. This is usually writeable, but will be read-only in
    /// some configurations.
    /// An optional follower database. Always read-only.
    pub url: String,
    #[serde(default = "default_db_pool_size")]
    pub pool_size: u32,
    pub min_idle: Option<u32>,

    /// Number of seconds to wait for unacknowledged TCP packets before treating the connection as
    /// broken. This value will determine how long crates.io stays unavailable in case of full
    /// packet loss between the application and the database: setting it too high will result in an
    /// unnecessarily long outage (before the unhealthy database logic kicks in), while setting it
    /// too low might result in healthy connections being dropped.
    #[serde(default = "default_tcp_timeout")]
    pub tcp_timeout: u64,
    /// Time to wait for a connection to become available from the connection
    /// pool before returning an error.
    /// Time to wait for a connection to become available from the connection
    /// pool before returning an error.
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout: u64,
    /// Time to wait for a query response before canceling the query and
    /// returning an error.
    #[serde(default = "default_statement_timeout")]
    pub statement_timeout: u64,
    /// Number of threads to use for asynchronous operations such as connection
    /// creation.
    #[serde(default = "default_helper_threads")]
    pub helper_threads: usize,
    /// Whether to enforce that all the database connections are encrypted with TLS.
    #[serde(default = "default_false")]
    pub enforce_tls: bool,
}

impl fmt::Display for DbConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Prepare a list of config values to show
        let lines = [
            ("tcp_timeout", self.tcp_timeout),
            // ("connection_timeout", &self.connection_timeout),
            // ("helper_threads", &self.helper_threads),
            // ("enforce_tls", self.enforce_tls.to_string()),
        ];

        let mut msg: String = "Active config values:\n\n".to_owned();

        for line in lines.into_iter().enumerate() {
            msg += &format!("{}: {}\n", line.1.0, line.1.1);
        }

        write!(f, "{msg}")
    }
}

// impl DbConfig {
//     const DEFAULT_POOL_SIZE: u32 = 1;

//     pub fn are_all_read_only(&self) -> bool {
//         self.primary.read_only_mode
//     }
// }

#[derive(Debug, Clone, Copy)]
pub struct ConnectionConfig {
    pub statement_timeout: u64,
    // pub read_only: bool,
}

impl CustomizeConnection<PgConnection, r2d2::Error> for ConnectionConfig {
    fn on_acquire(&self, conn: &mut PgConnection) -> Result<(), r2d2::Error> {
        use diesel::sql_query;

        // SAFETY: statement_timeout is a u64, which guarantees it's a valid integer.
        // PostgreSQL SET statements don't support parameterized queries, so we must
        // use string formatting. The u64 type ensures no SQL injection is possible
        // since it can only contain numeric digits.
        //
        // Validate the timeout is within a reasonable range (0 to 1 hour in ms)
        // to prevent potential DoS through extremely long timeouts.
        let timeout = self.statement_timeout.min(3_600_000);
        sql_query(format!("SET statement_timeout = {}", timeout))
            .execute(conn)
            .map_err(r2d2::Error::QueryError)?;
        // if self.read_only {
        //     sql_query("SET default_transaction_read_only = 't'")
        //         .execute(conn)
        //         .map_err(r2d2::Error::QueryError)?;
        // }
        Ok(())
    }
}
