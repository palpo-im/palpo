mod coordination;
mod preview;
mod remote;
use std::cmp;
use std::num::Saturating;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
pub use preview::*;
pub use remote::*;

use crate::core::http_headers::ContentDisposition;
use crate::core::media::ResizeMethod;
use crate::core::{Mxc, OwnedMxcUri, ServerName, UnixMillis, UserId};
use crate::data::connect;
use crate::data::media::{DbThumbnail, NewDbThumbnail};
use crate::data::schema::*;
use crate::{AppResult, config, data, storage};

#[derive(Debug)]
pub struct FileMeta {
    pub content: Option<Vec<u8>>,
    pub content_type: Option<String>,
    pub content_disposition: Option<ContentDisposition>,
}

/// Dimension specification for a thumbnail.
#[derive(Debug)]
pub struct Dimension {
    pub width: u32,
    pub height: u32,
    pub method: ResizeMethod,
}

impl Dimension {
    /// Instantiate a Dim with optional method
    #[inline]
    #[must_use]
    pub fn new(width: u32, height: u32, method: Option<ResizeMethod>) -> Self {
        Self {
            width,
            height,
            method: method.unwrap_or(ResizeMethod::Scale),
        }
    }

    pub fn scaled(&self, image: &Self) -> AppResult<Self> {
        let image_width = image.width;
        let image_height = image.height;

        let width = cmp::min(self.width, image_width);
        let height = cmp::min(self.height, image_height);

        let use_width = Saturating(width) * Saturating(image_height)
            < Saturating(height) * Saturating(image_width);

        let x = if use_width {
            let dividend = (Saturating(height) * Saturating(image_width)).0;
            dividend / image_height
        } else {
            width
        };

        let y = if !use_width {
            let dividend = (Saturating(width) * Saturating(image_height)).0;
            dividend / image_width
        } else {
            height
        };

        Ok(Self {
            width: x,
            height: y,
            method: ResizeMethod::Scale,
        })
    }

    /// Returns width, height of the thumbnail and whether it should be cropped.
    /// Returns None when the server should send the original file.
    /// Ignores the input Method.
    #[must_use]
    pub fn normalized(&self) -> Self {
        match (self.width, self.height) {
            (0..=32, 0..=32) => Self::new(32, 32, Some(ResizeMethod::Crop)),
            (0..=96, 0..=96) => Self::new(96, 96, Some(ResizeMethod::Crop)),
            (0..=320, 0..=240) => Self::new(320, 240, Some(ResizeMethod::Scale)),
            (0..=640, 0..=480) => Self::new(640, 480, Some(ResizeMethod::Scale)),
            (0..=800, 0..=600) => Self::new(800, 600, Some(ResizeMethod::Scale)),
            _ => Self::default(),
        }
    }

    /// Returns true if the method is Crop.
    #[inline]
    #[must_use]
    pub fn crop(&self) -> bool {
        self.method == ResizeMethod::Crop
    }
}

impl Default for Dimension {
    #[inline]
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            method: ResizeMethod::Scale,
        }
    }
}

/// Resolve the effective server_name string used in storage keys.
fn effective_server_name(server_name: &ServerName) -> &str {
    if server_name == config::server_name() {
        "_"
    } else {
        server_name.as_str()
    }
}

/// Build the storage key for a media file.
pub fn media_storage_key(server_name: &ServerName, media_id: &str) -> String {
    storage::media_key(effective_server_name(server_name), media_id)
}

pub async fn delete_media(server_name: &ServerName, media_id: &str) -> AppResult<()> {
    data::media::delete_media(server_name, media_id).await?;
    let key = media_storage_key(server_name, media_id);
    if let Err(e) = storage::delete(&key).await {
        tracing::error!("failed to delete media file '{key}': {e}");
    }
    Ok(())
}

/// Returns width, height of the thumbnail and whether it should be cropped. Returns None when
/// the server should send the original file.
pub fn thumbnail_properties(width: u32, height: u32) -> Option<(u32, u32, bool)> {
    match (width, height) {
        (0..=32, 0..=32) => Some((32, 32, true)),
        (0..=96, 0..=96) => Some((96, 96, true)),
        (0..=320, 0..=240) => Some((320, 240, false)),
        (0..=640, 0..=480) => Some((640, 480, false)),
        (0..=800, 0..=600) => Some((800, 600, false)),
        _ => None,
    }
}

/// Upper bound on `width` / `height` accepted from a thumbnail request.
///
/// `ThumbnailReqArgs::{width,height}` are `u32`, which means a remote can
/// ask for `4_294_967_295 x 4_294_967_295` and the server will at least
/// do a database lookup with those values before falling back. Clamp the
/// request at the boundary so neither the DB nor the image decoder has
/// to defend against absurd inputs.
pub const MAX_THUMBNAIL_DIMENSION: u32 = 2048;

/// Reject thumbnail requests whose declared width or height is implausibly
/// large. Returns `Err(M_INVALID_PARAM)` if either dimension exceeds
/// [`MAX_THUMBNAIL_DIMENSION`].
pub fn validate_thumbnail_dimensions(width: u32, height: u32) -> AppResult<()> {
    if width > MAX_THUMBNAIL_DIMENSION || height > MAX_THUMBNAIL_DIMENSION {
        return Err(crate::MatrixError::invalid_param(format!(
            "thumbnail dimensions must be <= {MAX_THUMBNAIL_DIMENSION} \
             (requested {width}x{height})"
        ))
        .into());
    }
    Ok(())
}

pub async fn get_all_mxcs() -> AppResult<Vec<OwnedMxcUri>> {
    let mxcs = media_metadatas::table
        .select((media_metadatas::origin_server, media_metadatas::media_id))
        .load::<(String, String)>(&mut connect().await?)
        .await?
        .into_iter()
        .map(|(origin_server, media_id)| {
            OwnedMxcUri::from(format!("mxc://{origin_server}/{media_id}"))
        })
        .collect();
    Ok(mxcs)
}

/// Save or replaces a file thumbnail.
#[allow(clippy::too_many_arguments)]
pub async fn save_thumbnail(
    mxc: &Mxc<'_>,
    _user: Option<&UserId>,
    content_type: Option<&str>,
    content_disposition: Option<&ContentDisposition>,
    dim: &Dimension,
    file: &[u8],
) -> AppResult<()> {
    let db_thumbnail = NewDbThumbnail {
        media_id: mxc.media_id.to_owned(),
        origin_server: mxc.server_name.to_owned(),
        content_type: content_type.map(|c| c.to_owned()),
        disposition_type: content_disposition.map(|d| d.to_string()),
        file_size: file.len() as i64,
        width: dim.width as i32,
        height: dim.height as i32,
        resize_method: dim.method.to_string(),
        created_at: UnixMillis::now(),
    };
    publish_thumbnail(db_thumbnail, file).await?;

    Ok(())
}

/// Build the storage key for a thumbnail file.
pub fn thumbnail_storage_key(
    server_name: &ServerName,
    media_id: &str,
    thumbnail_id: i64,
) -> String {
    storage::thumbnail_key(effective_server_name(server_name), media_id, thumbnail_id)
}

pub async fn save_thumbnail_file(
    server_name: &ServerName,
    media_id: &str,
    thumbnail_id: i64,
    file: &[u8],
) -> AppResult<String> {
    let key = thumbnail_storage_key(server_name, media_id, thumbnail_id);
    storage::write(&key, file).await?;
    Ok(key)
}

/// Missing cache objects from interrupted older writes must not remain cache hits.
pub async fn cached_thumbnail(
    origin: &ServerName,
    media_id: &str,
    width: u32,
    height: u32,
) -> AppResult<Option<DbThumbnail>> {
    let Some(thumbnail) =
        data::media::get_thumbnail_by_dimension(origin, media_id, width, height).await?
    else {
        return Ok(None);
    };
    validate_cached_thumbnail(
        storage::operator(),
        effective_server_name(origin),
        thumbnail,
    )
    .await
}

async fn validate_cached_thumbnail(
    operator: &opendal::Operator,
    storage_origin: &str,
    thumbnail: DbThumbnail,
) -> AppResult<Option<DbThumbnail>> {
    let origin = thumbnail.origin_server.clone();
    let media_id = thumbnail.media_id.clone();
    coordination::with_lock(&origin, &media_id, async {
        let key = storage::thumbnail_key(storage_origin, &thumbnail.media_id, thumbnail.id);
        if operator.exists(&key).await? {
            return Ok(Some(thumbnail));
        }
        // Only NotFound invalidates metadata. Permission/network errors propagate above.
        diesel::delete(media_thumbnails::table.filter(media_thumbnails::id.eq(thumbnail.id)))
            .execute(&mut connect().await?)
            .await?;
        Ok(None)
    })
    .await
}

pub(crate) async fn publish_thumbnail(record: NewDbThumbnail, bytes: &[u8]) -> AppResult<i64> {
    let storage_origin = effective_server_name(&record.origin_server).to_owned();
    let require_original = record.origin_server == config::server_name();
    publish_thumbnail_with_operator(
        storage::operator(),
        &storage_origin,
        record,
        bytes,
        require_original,
    )
    .await
}

async fn publish_thumbnail_with_operator(
    operator: &opendal::Operator,
    storage_origin: &str,
    record: NewDbThumbnail,
    bytes: &[u8],
    require_original: bool,
) -> AppResult<i64> {
    let origin = record.origin_server.clone();
    let media_id = record.media_id.clone();
    coordination::with_lock(&origin, &media_id, async {
        publish_thumbnail_locked(operator, storage_origin, record, bytes, require_original).await
    })
    .await
}

async fn publish_thumbnail_locked(
    operator: &opendal::Operator,
    storage_origin: &str,
    record: NewDbThumbnail,
    bytes: &[u8],
    require_original: bool,
) -> AppResult<i64> {
    // Local CPU work can finish after a concurrent deletion. Never republish the
    // captured image once that deletion has removed its original. Remote-only
    // thumbnail caches intentionally do not require an original on this server.
    if require_original
        && (data::media::get_metadata(&record.origin_server, &record.media_id)
            .await?
            .is_none()
            || !operator
                .exists(&storage::media_key(storage_origin, &record.media_id))
                .await?)
    {
        return Err(crate::MatrixError::not_found(
            "Original media was deleted before thumbnail publication.",
        )
        .into());
    }
    for _ in 0..3 {
        let existing = media_thumbnails::table
            .filter(media_thumbnails::origin_server.eq(&record.origin_server))
            .filter(media_thumbnails::media_id.eq(&record.media_id))
            .filter(media_thumbnails::width.eq(record.width))
            .filter(media_thumbnails::height.eq(record.height))
            .filter(media_thumbnails::resize_method.eq(&record.resize_method))
            .first::<DbThumbnail>(&mut connect().await?)
            .await
            .optional()?;
        if let Some(existing) = existing {
            let key = storage::thumbnail_key(storage_origin, &record.media_id, existing.id);
            if operator.exists(&key).await? {
                return Ok(existing.id);
            }
            // Heal an exact-key winner left by an older interrupted publication.
            diesel::delete(media_thumbnails::table.filter(media_thumbnails::id.eq(existing.id)))
                .execute(&mut connect().await?)
                .await?;
        }
        // Reserve the ID without publishing metadata until the object exists.
        let id = diesel::dsl::sql::<diesel::sql_types::BigInt>(
            "SELECT nextval(pg_get_serial_sequence('media_thumbnails', 'id'))",
        )
        .get_result::<i64>(&mut connect().await?)
        .await?;
        let key = storage::thumbnail_key(storage_origin, &record.media_id, id);
        operator.write(&key, bytes.to_vec()).await?;
        let inserted = diesel::insert_into(media_thumbnails::table)
            .values((media_thumbnails::id.eq(id), &record))
            .on_conflict_do_nothing()
            .returning(media_thumbnails::id)
            .get_result::<i64>(&mut connect().await?)
            .await
            .optional()?;
        if let Some(id) = inserted {
            return Ok(id);
        }
        // A non-cooperating old process may still race. Remove only our known
        // unused object, then recheck the winner's file on the next iteration.
        // On ambiguous SQL error the ? above keeps the possibly published file.
        operator.delete(&key).await?;
    }
    Err(crate::AppError::internal(
        "Thumbnail cache changed repeatedly during publication",
    ))
}

pub(crate) async fn find_cached_thumbnail_id(
    origin: &ServerName,
    media_id: &str,
    width: u32,
    height: u32,
    method: &str,
) -> AppResult<i64> {
    Ok(media_thumbnails::table
        .filter(media_thumbnails::origin_server.eq(origin))
        .filter(media_thumbnails::media_id.eq(media_id))
        .filter(media_thumbnails::width.eq(width as i32))
        .filter(media_thumbnails::height.eq(height as i32))
        .filter(media_thumbnails::resize_method.eq(method))
        .select(media_thumbnails::id)
        .first::<i64>(&mut connect().await?)
        .await?)
}

#[cfg(test)]
mod thumbnail_publication_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn storage_failure_and_missing_legacy_thumbnail_can_be_retried() {
        crate::test_database::init();
        let dir = tempfile::tempdir().unwrap();
        let operator = opendal::Operator::new(
            opendal::services::Fs::default().root(dir.path().to_str().unwrap()),
        )
        .unwrap();
        let media_id = format!("q18-{}", uuid::Uuid::new_v4());
        let record = NewDbThumbnail {
            media_id: media_id.clone(),
            origin_server: ServerName::parse("q18.test").unwrap(),
            content_type: Some("image/png".into()),
            disposition_type: None,
            file_size: 3,
            width: 96,
            height: 96,
            resize_method: "scale".into(),
            created_at: UnixMillis::now(),
        };
        // Make the expected object prefix a file so the first write deterministically fails.
        operator
            .write("media", b"not a directory".to_vec())
            .await
            .unwrap();
        assert!(
            publish_thumbnail_with_operator(&operator, "_", record.clone(), b"png", false)
                .await
                .is_err()
        );
        assert!(
            data::media::get_thumbnail_by_dimension(&record.origin_server, &media_id, 96, 96)
                .await
                .unwrap()
                .is_none()
        );
        operator.delete("media").await.unwrap();
        let id = publish_thumbnail_with_operator(&operator, "_", record.clone(), b"png", false)
            .await
            .unwrap();
        let row = data::media::get_thumbnail_by_dimension(&record.origin_server, &media_id, 96, 96)
            .await
            .unwrap()
            .unwrap();
        assert!(
            validate_cached_thumbnail(&operator, "_", row.clone())
                .await
                .unwrap()
                .is_some()
        );
        // Simulate a legacy cache row committed before the interrupted object write.
        operator
            .delete(&storage::thumbnail_key("_", &media_id, id))
            .await
            .unwrap();
        assert!(
            validate_cached_thumbnail(&operator, "_", row)
                .await
                .unwrap()
                .is_none()
        );
        let next = publish_thumbnail_with_operator(&operator, "_", record.clone(), b"new", false)
            .await
            .unwrap();
        assert_ne!(id, next);
        assert_eq!(
            operator
                .read(&storage::thumbnail_key("_", &media_id, next))
                .await
                .unwrap()
                .to_vec(),
            b"new"
        );
        let (left, right) = tokio::join!(
            publish_thumbnail_with_operator(&operator, "_", record.clone(), b"new", false),
            publish_thumbnail_with_operator(&operator, "_", record, b"new", false),
        );
        assert_eq!(left.unwrap(), next);
        assert_eq!(right.unwrap(), next);
        diesel::delete(media_thumbnails::table.filter(media_thumbnails::id.eq(next)))
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn stale_winner_is_repaired_and_delayed_local_publication_does_not_resurrect_media() {
        crate::test_database::init();
        let directory = tempfile::tempdir().unwrap();
        let operator = opendal::Operator::new(
            opendal::services::Fs::default().root(directory.path().to_str().unwrap()),
        )
        .unwrap();
        let origin = ServerName::parse("q18-fence.test").unwrap();
        let media_id = format!("q18-fence-{}", uuid::Uuid::new_v4());
        let record = NewDbThumbnail {
            media_id: media_id.clone(),
            origin_server: origin.clone(),
            content_type: Some("image/png".into()),
            disposition_type: None,
            file_size: 3,
            width: 96,
            height: 96,
            resize_method: "scale".into(),
            created_at: UnixMillis::now(),
        };
        // Remote-only thumbnail caches remain supported without original metadata.
        let missing_id = diesel::insert_into(media_thumbnails::table)
            .values(&record)
            .returning(media_thumbnails::id)
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        let valid_id = publish_thumbnail_with_operator(
            &operator,
            "q18-fence.test",
            record.clone(),
            b"png",
            false,
        )
        .await
        .unwrap();
        assert_ne!(valid_id, missing_id);
        assert!(
            operator
                .exists(&storage::thumbnail_key(
                    "q18-fence.test",
                    &media_id,
                    valid_id
                ))
                .await
                .unwrap()
        );
        // Local publication must observe deletion while waiting for the same DB fence.
        data::media::insert_metadata(&data::media::NewDbMetadata {
            media_id: media_id.clone(),
            origin_server: origin.clone(),
            content_type: None,
            disposition_type: None,
            file_name: None,
            file_extension: None,
            file_size: 4,
            file_hash: None,
            created_by: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        let original = storage::media_key("_", &media_id);
        operator.write(&original, b"file".to_vec()).await.unwrap();
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let (delete_origin, delete_id, delete_operator) =
            (origin.clone(), media_id.clone(), operator.clone());
        let deletion = tokio::spawn(async move {
            coordination::with_lock(&delete_origin, &delete_id, async {
                let _ = locked_tx.send(());
                finish_rx.await.unwrap();
                // Run the same storage/metadata deletion used by the Q06 service,
                // while the test controls the moment it releases the shared fence.
                coordination::delete_locked(&delete_operator, "_", &delete_origin, &delete_id)
                    .await?;
                Ok(())
            })
            .await
        });
        locked_rx.await.unwrap();
        let (publish_operator, publish_record) = (operator.clone(), record.clone());
        let publisher = tokio::spawn(async move {
            publish_thumbnail_with_operator(&publish_operator, "_", publish_record, b"png", true)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !publisher.is_finished(),
            "publisher bypassed deletion fence"
        );
        finish_tx.send(()).unwrap();
        deletion.await.unwrap().unwrap();
        assert!(publisher.await.unwrap().is_err());
        assert!(!operator.exists(&original).await.unwrap());
        assert!(
            data::media::get_thumbnail_by_dimension(&origin, &media_id, 96, 96)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            operator
                .list(&format!(
                    "{}.thumbnails/",
                    storage::media_key("_", &media_id)
                ))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
