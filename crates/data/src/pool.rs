use std::time::Duration;

use deadpool::Runtime;
use deadpool::managed::{Status, Timeouts};
use diesel::ConnectionError;
use diesel_async::pooled_connection::deadpool::{
    BuildError, Object, Pool, PoolError as DeadpoolError,
};
use diesel_async::pooled_connection::{AsyncDieselConnectionManager, ManagerConfig};
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::FutureExt;
use thiserror::Error;

use super::{DbConfig, connection_url};

pub type PgPool = Pool<AsyncPgConnection>;
pub type PgPooledConnection = Object<AsyncPgConnection>;

#[derive(Clone)]
pub struct DieselPool {
    inner: PgPool,
}

impl std::fmt::Debug for DieselPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselPool").finish_non_exhaustive()
    }
}

impl DieselPool {
    pub(crate) fn new(url: &str, config: &DbConfig) -> Result<DieselPool, PoolError> {
        let conn_url = connection_url(config, url);

        // PostgreSQL `SET` does not support bind parameters, so the value is
        // formatted into the statement. `statement_timeout` is a `u64` clamped to
        // one hour, so no injection is possible.
        let statement_timeout = config.statement_timeout.min(3_600_000);

        let mut manager_config = ManagerConfig::default();
        manager_config.custom_setup = Box::new(move |url: &str| {
            let url = url.to_owned();
            async move {
                let mut conn = AsyncPgConnection::establish(&url).await?;
                diesel::sql_query(format!("SET statement_timeout = {statement_timeout}"))
                    .execute(&mut conn)
                    .await
                    .map_err(ConnectionError::CouldntSetupConfiguration)?;
                Ok(conn)
            }
            .boxed()
        });

        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(
            conn_url,
            manager_config,
        );

        let timeout = Duration::from_millis(config.connection_timeout);
        let inner = Pool::builder(manager)
            .max_size(config.pool_size as usize)
            .runtime(Runtime::Tokio1)
            .timeouts(Timeouts {
                wait: Some(timeout),
                create: Some(timeout),
                recycle: Some(timeout),
            })
            .build()?;

        tracing::info!("Database pool is created");
        Ok(DieselPool { inner })
    }

    pub async fn get(&self) -> Result<PgPooledConnection, PoolError> {
        Ok(self.inner.get().await?)
    }

    pub fn status(&self) -> Status {
        self.inner.status()
    }
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Get(#[from] DeadpoolError),
}
