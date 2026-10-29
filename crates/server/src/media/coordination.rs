//! Cross-instance fence for a media object's storage and metadata lifecycle.
use std::future::Future;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::ServerName;
use crate::data::{connect, schema::*};
use crate::{AppError, AppResult, data, storage};

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

/// Shared deletion implementation so the publication and deletion PRs exercise
/// the same lifecycle protocol even when reviewed independently.
pub(super) async fn delete_media(
    operator: &opendal::Operator,
    storage_origin: &str,
    server_name: &ServerName,
    media_id: &str,
) -> AppResult<()> {
    with_lock(server_name, media_id, async {
        delete_locked(operator, storage_origin, server_name, media_id).await
    })
    .await
}

/// The caller must hold this media's advisory fence for the whole operation.
pub(super) async fn delete_locked(
    operator: &opendal::Operator,
    storage_origin: &str,
    server_name: &ServerName,
    media_id: &str,
) -> AppResult<()> {
    // Include objects left unreferenced by interrupted thumbnail publication.
    let key = storage::media_key(storage_origin, media_id);
    operator.remove_all(&format!("{key}.thumbnails/")).await?;
    // Older federation code stored dimension-keyed files. Delete only dimensions
    // represented in this media's rows, never an arbitrary media-ID prefix.
    let dimensions = media_thumbnails::table
        .filter(media_thumbnails::origin_server.eq(server_name))
        .filter(media_thumbnails::media_id.eq(media_id))
        .select((media_thumbnails::width, media_thumbnails::height))
        .distinct()
        .load::<(i32, i32)>(&mut connect().await?)
        .await?;
    for (width, height) in dimensions {
        let legacy_id = format!("{media_id}.{width}x{height}");
        // A legitimate separately uploaded media ID takes precedence over an
        // ambiguous old cache filename.
        if data::media::get_metadata(server_name, &legacy_id)
            .await?
            .is_none()
        {
            operator
                .delete(&storage::media_key(storage_origin, &legacy_id))
                .await?;
        }
    }
    operator.delete(&key).await?;
    // Never discard cleanup identities if ANY object deletion failed above.
    connect()
        .await?
        .transaction::<_, diesel::result::Error, _>(async |connection| {
            diesel::delete(
                media_thumbnails::table
                    .filter(media_thumbnails::origin_server.eq(server_name))
                    .filter(media_thumbnails::media_id.eq(media_id)),
            )
            .execute(connection)
            .await?;
            diesel::delete(
                media_metadatas::table
                    .filter(media_metadatas::origin_server.eq(server_name))
                    .filter(media_metadatas::media_id.eq(media_id)),
            )
            .execute(connection)
            .await?;
            Ok(())
        })
        .await?;
    Ok(())
}
