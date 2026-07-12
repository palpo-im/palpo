use serde::Deserialize;

use crate::core::serde::default_false;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "db")]
#[derive(Clone, Debug, Deserialize)]
pub struct DbConfig {
    /// Settings for the primary database. default reade env var PALPO_DB_URL.
    #[serde(default = "default_db_url")]
    pub url: String,
    #[serde(default = "default_db_pool_size")]
    pub pool_size: u32,

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
    /// Whether to enforce that all the database connections are encrypted with TLS.
    #[serde(default = "default_false")]
    pub enforce_tls: bool,
}

impl DbConfig {
    pub fn into_data_db_config(self) -> crate::data::DbConfig {
        let Self {
            url,
            pool_size,
            tcp_timeout,
            connection_timeout,
            statement_timeout,
            enforce_tls,
        } = self;
        crate::data::DbConfig {
            url: url.clone(),
            pool_size,
            tcp_timeout,
            connection_timeout,
            statement_timeout,
            enforce_tls,
        }
    }
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            pool_size: default_db_pool_size(),
            tcp_timeout: default_tcp_timeout(),
            connection_timeout: default_connection_timeout(),
            statement_timeout: default_statement_timeout(),
            enforce_tls: default_false(),
        }
    }
}

fn default_db_url() -> String {
    std::env::var("PALPO_DB_URL").unwrap_or_default()
}

fn default_db_pool_size() -> u32 {
    10
}
fn default_tcp_timeout() -> u64 {
    10_000
}
fn default_connection_timeout() -> u64 {
    30_000
}
fn default_statement_timeout() -> u64 {
    30_000
}
