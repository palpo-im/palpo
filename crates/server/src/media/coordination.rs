//! Cross-instance fence for a media object's storage and metadata lifecycle.
use std::future::Future;

use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::ServerName;
use crate::{AppError, AppResult, data};

const MEDIA_LOCK_NAMESPACE: i64 = 7_252_664_865;

/// The lock connection comes from the coordination pool. The operation may use
/// ordinary query connections without pinning the only connection it needs.
/// A transaction lock is released on completion, cancellation or connection loss.
pub(super) async fn with_lock<T, F>(
    origin: &ServerName,
    media_id: &str,
    operation: F,
) -> AppResult<T>
where
    F: Future<Output = AppResult<T>> + Send,
    T: Send,
{
    let key = format!("{}:{origin}{media_id}", origin.as_str().len());
    let mut connection = data::coordination_connect().await?;
    connection
        .transaction::<_, AppError, _>(async |connection| {
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, $2))")
                .bind::<diesel::sql_types::Text, _>(&key)
                .bind::<diesel::sql_types::BigInt, _>(MEDIA_LOCK_NAMESPACE)
                .execute(connection)
                .await?;
            operation.await
        })
        .await
}
