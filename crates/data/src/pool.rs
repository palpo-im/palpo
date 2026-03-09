use std::ops::Deref;
use std::time::Duration;

use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager, State};
use thiserror::Error;

use super::{DbConfig, connection_url};

pub type PgPool = r2d2::Pool<ConnectionManager<PgConnection>>;
pub type PgPooledConnection = r2d2::PooledConnection<ConnectionManager<PgConnection>>;

#[derive(Clone, Debug)]
pub struct DieselPool {
    inner: PgPool,
}

impl DieselPool {
    pub(crate) fn new(
        url: &str,
        config: &DbConfig,
        r2d2_config: r2d2::Builder<ConnectionManager<PgConnection>>,
    ) -> Result<DieselPool, PoolError> {
        let manager = ConnectionManager::new(connection_url(config, url));

        let pool = DieselPool {
            inner: r2d2_config.build_unchecked(manager),
        };
        match pool.wait_until_healthy(Duration::from_secs(10)) {
            Ok(()) => {
                tracing::info!("Database pool is healthy");
            }
            Err(PoolError::UnhealthyPool) => {
                tracing::error!("Database pool is unhealthy");
            }
            Err(e) => {
                tracing::error!("Database pool is unhealthy: {e}");
                return Err(e);
            }
        }
        tracing::info!("Database pool is created");

        Ok(pool)
    }

    pub fn new_background_worker(inner: r2d2::Pool<ConnectionManager<PgConnection>>) -> Self {
        Self { inner }
    }

    pub fn get(&self) -> Result<PgPooledConnection, PoolError> {
        Ok(self.inner.get()?)
    }

    pub fn state(&self) -> State {
        self.inner.state()
    }

    pub fn wait_until_healthy(&self, timeout: Duration) -> Result<(), PoolError> {
        match self.inner.get_timeout(timeout) {
            Ok(_) => Ok(()),
            Err(_) if !self.is_healthy() => Err(PoolError::UnhealthyPool),
            Err(err) => Err(PoolError::R2D2(err)),
        }
    }

    fn is_healthy(&self) -> bool {
        let state = self.state();
        state.connections > 0 && state.idle_connections > 0
    }
}

impl Deref for DieselPool {
    type Target = PgPool;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error(transparent)]
    R2D2(#[from] r2d2::PoolError),
    #[error("unhealthy database pool")]
    UnhealthyPool,
    #[error("Failed to lock test database connection")]
    TestConnectionUnavailable,
}
