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

/// An inexpensive query provider. Holding it never reserves a pool connection.
#[derive(Clone, Debug)]
pub struct Db {
    pool: DieselPool,
}

impl Db {
    /// Hold one lease for the transaction; pass its connection to nested helpers.
    #[allow(
        clippy::manual_async_fn,
        reason = "Nested borrowed callbacks need the explicit Send + 'conn future bound"
    )]
    pub fn transaction<'a, 'conn, T, E, F>(
        &'conn mut self,
        callback: F,
    ) -> impl std::future::Future<Output = Result<T, E>> + Send + 'conn
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncPgConnection) -> Result<T, E>
            + horme::TransactionCallback<&'r mut AsyncPgConnection, Result<T, E>>
            + Send
            + 'a,
        T: Send + 'a,
        E: From<horme::Error> + From<diesel::result::Error> + Send + 'a,
        'a: 'conn,
    {
        async move {
            let lease = horme::GetConnection::get_mut_conn(self).await?;
            let mut transaction = TransactionConnection::new(lease);
            let result = horme::transaction(transaction.connection(), callback).await;
            transaction.mark_finished();
            result
        }
    }

    /// Configure a transaction without checking out a connection until `run`.
    pub fn build_transaction(&mut self) -> TransactionBuilder<'_> {
        TransactionBuilder {
            db: self,
            read_only: false,
            isolation: None,
        }
    }
}

/// A transaction configuration which holds only a borrowed pool handle.
pub struct TransactionBuilder<'db> {
    db: &'db mut Db,
    read_only: bool,
    isolation: Option<IsolationLevel>,
}

enum IsolationLevel {
    RepeatableRead,
    ReadCommitted,
}

impl TransactionBuilder<'_> {
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn repeatable_read(mut self) -> Self {
        self.isolation = Some(IsolationLevel::RepeatableRead);
        self
    }

    pub fn read_committed(mut self) -> Self {
        self.isolation = Some(IsolationLevel::ReadCommitted);
        self
    }

    #[allow(
        clippy::manual_async_fn,
        reason = "Nested borrowed callbacks need the explicit Send + 'conn future bound"
    )]
    pub fn run<'a, 'conn, T, E, F>(
        &'conn mut self,
        callback: F,
    ) -> impl std::future::Future<Output = Result<T, E>> + Send + 'conn
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncPgConnection) -> Result<T, E>
            + horme::TransactionCallback<&'r mut AsyncPgConnection, Result<T, E>>
            + Send
            + 'a,
        T: Send + 'a,
        E: From<horme::Error> + From<diesel::result::Error> + Send + 'a,
        'a: 'conn,
    {
        async move {
            let lease = horme::GetConnection::get_mut_conn(self.db).await?;
            let mut transaction = TransactionConnection::new(lease);
            let mut builder = transaction.connection().build_transaction();
            if self.read_only {
                builder = builder.read_only();
            }
            builder = match self.isolation {
                Some(IsolationLevel::RepeatableRead) => builder.repeatable_read(),
                Some(IsolationLevel::ReadCommitted) => builder.read_committed(),
                None => builder,
            };
            let result = builder.run(callback).await;
            transaction.mark_finished();
            result
        }
    }
}

/// Own a transaction's lease until commit/rollback has completed. Cancellation,
/// including during BEGIN, closes its backend immediately instead of returning
/// a connection with uncertain lock state to the idle pool.
#[doc(hidden)]
pub struct TransactionConnection {
    connection: Option<PgPooledConnection>,
    finished: bool,
}

impl TransactionConnection {
    pub fn new(connection: PgPooledConnection) -> Self {
        Self {
            connection: Some(connection),
            finished: false,
        }
    }

    pub fn connection(&mut self) -> &mut AsyncPgConnection {
        self.connection
            .as_mut()
            .expect("transaction connection is present")
    }

    pub fn mark_finished(&mut self) {
        self.finished = true;
    }
}

impl Drop for TransactionConnection {
    fn drop(&mut self) {
        let Some(mut connection) = self.connection.take() else {
            return;
        };
        if !self.finished
            || diesel_async::pooled_connection::PoolableConnection::is_broken(&mut *connection)
        {
            drop(PgPooledConnection::take(connection));
        }
    }
}

impl horme::GetConnection for Db {
    type Connection = AsyncPgConnection;
    type Lease<'a> = PgPooledConnection;

    async fn get_mut_conn(&mut self) -> horme::Result<Self::Lease<'_>> {
        let wait = self.pool.wait_metrics.start();
        let result = self.pool.checkout().await;
        wait.finish(result.is_ok());
        result.map_err(|error| {
            tracing::error!(%error, coordination = self.pool.coordination, "db connect error");
            horme::Error::Acquire(Box::new(error))
        })
    }
}

#[derive(Clone)]
pub struct DieselPool {
    inner: PgPool,
    wait_metrics: &'static super::telemetry::WaitMetrics,
    coordination: bool,
}

impl std::fmt::Debug for DieselPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DieselPool").finish_non_exhaustive()
    }
}

impl DieselPool {
    pub fn db(&self) -> Db {
        Db { pool: self.clone() }
    }

    pub(crate) fn new(
        url: &str,
        config: &DbConfig,
        max_size: usize,
        purpose: &str,
    ) -> Result<DieselPool, PoolError> {
        let conn_url = connection_url(config, url)?;

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
            .max_size(max_size)
            .runtime(Runtime::Tokio1)
            .timeouts(Timeouts {
                wait: Some(timeout),
                create: Some(timeout),
                recycle: Some(timeout),
            })
            .build()?;

        tracing::info!(max_size, purpose, "Database pool is created");
        let coordination = purpose == "coordination";
        let wait_metrics = if coordination {
            &super::telemetry::COORDINATION_POOL_WAIT
        } else {
            &super::telemetry::QUERY_POOL_WAIT
        };
        Ok(DieselPool {
            inner,
            wait_metrics,
            coordination,
        })
    }

    /// Explicit session lease for instrumentation and database coordination.
    /// Business queries and transactions use `db()` instead.
    pub(crate) async fn checkout(&self) -> Result<PgPooledConnection, PoolError> {
        Ok(self.inner.get().await?)
    }

    pub fn status(&self) -> Status {
        self.inner.status()
    }
}

#[derive(Debug, Error)]
pub enum PoolError {
    #[error(transparent)]
    Acquire(horme::Error),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error(transparent)]
    Build(#[from] BuildError),
    #[error(transparent)]
    Get(#[from] DeadpoolError),
}

#[cfg(test)]
mod tests;
