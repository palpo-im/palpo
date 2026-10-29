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
use crate::data::media::NewDbThumbnail;
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

/// Delete stored objects before discarding the metadata needed for retry.
/// Storage failures propagate; retrying after an error or restart is idempotent.
pub async fn delete_media(server_name: &ServerName, media_id: &str) -> AppResult<()> {
    delete_media_with_operator(
        storage::operator(),
        effective_server_name(server_name),
        server_name,
        media_id,
    )
    .await
}

async fn delete_media_with_operator(
    operator: &opendal::Operator,
    storage_origin: &str,
    server_name: &ServerName,
    media_id: &str,
) -> AppResult<()> {
    coordination::delete_media(operator, storage_origin, server_name, media_id).await
}

pub async fn delete_media_by_ids(
    server_name: &ServerName,
    media_ids: &[String],
) -> AppResult<(Vec<String>, i64)> {
    let mut deleted = Vec::new();
    for media_id in media_ids {
        if data::media::get_metadata(server_name, media_id)
            .await?
            .is_some()
        {
            delete_media(server_name, media_id).await?;
            deleted.push(media_id.clone());
        }
    }
    let total = deleted.len() as i64;
    Ok((deleted, total))
}

pub async fn delete_old_local_media(
    server_name: &ServerName,
    before_ts: i64,
    size_gt: i64,
) -> AppResult<(Vec<String>, i64)> {
    let media_ids = media_metadatas::table
        .filter(media_metadatas::origin_server.eq(server_name))
        .filter(media_metadatas::created_at.lt(UnixMillis(before_ts as u64)))
        .filter(media_metadatas::file_size.gt(size_gt))
        .select(media_metadatas::media_id)
        .load::<String>(&mut connect().await?)
        .await?;
    delete_media_by_ids(server_name, &media_ids).await
}

pub async fn purge_remote_media_cache(local_server: &ServerName, before_ts: i64) -> AppResult<i64> {
    let originals = media_metadatas::table
        .filter(media_metadatas::origin_server.ne(local_server))
        .filter(media_metadatas::created_at.lt(UnixMillis(before_ts as u64)))
        .select((media_metadatas::origin_server, media_metadatas::media_id))
        .load::<(crate::core::OwnedServerName, String)>(&mut connect().await?)
        .await?;
    let mut deleted = 0;
    for (origin, media_id) in originals {
        delete_media(&origin, &media_id).await?;
        deleted += 1;
    }
    // Old thumbnails of newer originals can be evicted without removing the original.
    let thumbnails = media_thumbnails::table
        .filter(media_thumbnails::origin_server.ne(local_server))
        .filter(media_thumbnails::created_at.lt(UnixMillis(before_ts as u64)))
        .load::<data::media::DbThumbnail>(&mut connect().await?)
        .await?;
    for thumbnail in thumbnails {
        coordination::with_lock(&thumbnail.origin_server, &thumbnail.media_id, async {
            let key =
                thumbnail_storage_key(&thumbnail.origin_server, &thumbnail.media_id, thumbnail.id);
            storage::delete(&key).await?;
            let legacy_id = format!(
                "{}.{}x{}",
                thumbnail.media_id, thumbnail.width, thumbnail.height
            );
            if data::media::get_metadata(&thumbnail.origin_server, &legacy_id)
                .await?
                .is_none()
            {
                storage::delete(&media_storage_key(&thumbnail.origin_server, &legacy_id)).await?;
            }
            diesel::delete(media_thumbnails::table.filter(media_thumbnails::id.eq(thumbnail.id)))
                .execute(&mut connect().await?)
                .await?;
            Ok(())
        })
        .await?;
        deleted += 1;
    }
    Ok(deleted)
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
    let id = diesel::insert_into(media_thumbnails::table)
        .values(&db_thumbnail)
        .on_conflict_do_nothing()
        .returning(media_thumbnails::id)
        .get_result::<i64>(&mut connect().await?)
        .await
        .optional()?;

    if let Some(id) = id {
        save_thumbnail_file(mxc.server_name, mxc.media_id, id, file).await?;
    }

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

#[cfg(test)]
mod deletion_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn deletion_removes_objects_and_retains_metadata_until_storage_succeeds() {
        crate::test_database::init();
        let directory = tempfile::tempdir().unwrap();
        let operator = opendal::Operator::new(
            opendal::services::Fs::default().root(directory.path().to_str().unwrap()),
        )
        .unwrap();
        let origin = ServerName::parse("q06.test").unwrap();
        let media_id = format!("q06-{}", uuid::Uuid::new_v4());
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
        let thumbnail_id = diesel::insert_into(media_thumbnails::table)
            .values(NewDbThumbnail {
                media_id: media_id.clone(),
                origin_server: origin.clone(),
                content_type: None,
                disposition_type: None,
                file_size: 5,
                width: 96,
                height: 96,
                resize_method: "scale".into(),
                created_at: UnixMillis::now(),
            })
            .returning(media_thumbnails::id)
            .get_result::<i64>(&mut connect().await.unwrap())
            .await
            .unwrap();
        let original = storage::media_key("_", &media_id);
        let thumbnail = storage::thumbnail_key("_", &media_id, thumbnail_id);
        let orphan = storage::thumbnail_key("_", &media_id, thumbnail_id + 1);
        let legacy = storage::media_key("_", &format!("{media_id}.96x96"));
        let other_origin = storage::media_key("other.test", &media_id);
        let other_media = storage::media_key("_", &format!("{media_id}-other"));
        for key in [&thumbnail, &orphan, &legacy, &other_origin, &other_media] {
            operator.write(key, b"bytes".to_vec()).await.unwrap();
        }
        // A nonempty directory at the original file key deterministically fails DELETE.
        operator
            .write(&format!("{original}/blocker"), b"fail".to_vec())
            .await
            .unwrap();
        assert!(
            delete_media_with_operator(&operator, "_", &origin, &media_id)
                .await
                .is_err()
        );
        assert!(
            data::media::get_metadata(&origin, &media_id)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            data::media::get_thumbnail_by_dimension(&origin, &media_id, 96, 96)
                .await
                .unwrap()
                .is_some()
        );
        // Storage was partly removed, but persistent DB identities survive a new attempt.
        operator
            .delete_with(&format!("{original}/"))
            .recursive(true)
            .await
            .unwrap();
        operator.write(&original, b"file".to_vec()).await.unwrap();
        let restarted_operator = opendal::Operator::new(
            opendal::services::Fs::default().root(directory.path().to_str().unwrap()),
        )
        .unwrap();
        delete_media_with_operator(&restarted_operator, "_", &origin, &media_id)
            .await
            .unwrap();
        delete_media_with_operator(&restarted_operator, "_", &origin, &media_id)
            .await
            .unwrap();
        for key in [&original, &thumbnail, &orphan, &legacy] {
            assert!(!operator.exists(key).await.unwrap());
        }
        for key in [&other_origin, &other_media] {
            assert!(operator.exists(key).await.unwrap());
        }
        assert!(
            data::media::get_metadata(&origin, &media_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            data::media::get_thumbnail_by_dimension(&origin, &media_id, 96, 96)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn deletion_waits_for_cross_instance_thumbnail_publication() {
        crate::test_database::init();
        let directory = tempfile::tempdir().unwrap();
        let operator = opendal::Operator::new(
            opendal::services::Fs::default().root(directory.path().to_str().unwrap()),
        )
        .unwrap();
        let origin = ServerName::parse("q06-fence.test").unwrap();
        let media_id = format!("q06-fence-{}", uuid::Uuid::new_v4());
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
        let original_key = storage::media_key("_", &media_id);
        operator
            .write(&original_key, b"file".to_vec())
            .await
            .unwrap();
        let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (write_origin, write_id, write_operator) =
            (origin.clone(), media_id.clone(), operator.clone());
        let publisher = tokio::spawn(async move {
            coordination::with_lock(&write_origin, &write_id, async {
                let _ = acquired_tx.send(());
                release_rx.await.unwrap();
                // Exercise the publication protocol using a different coordination
                // connection from the real deletion service below.
                let id = diesel::insert_into(media_thumbnails::table)
                    .values(NewDbThumbnail {
                        media_id: write_id.clone(),
                        origin_server: write_origin.clone(),
                        content_type: None,
                        disposition_type: None,
                        file_size: 3,
                        width: 96,
                        height: 96,
                        resize_method: "scale".into(),
                        created_at: UnixMillis::now(),
                    })
                    .returning(media_thumbnails::id)
                    .get_result::<i64>(&mut connect().await?)
                    .await?;
                let thumbnail_key = storage::thumbnail_key("_", &write_id, id);
                write_operator
                    .write(&thumbnail_key, b"png".to_vec())
                    .await?;
                Ok(thumbnail_key)
            })
            .await
        });
        acquired_rx.await.unwrap();
        let (delete_origin, delete_id, delete_operator) =
            (origin.clone(), media_id.clone(), operator.clone());
        let deletion = tokio::spawn(async move {
            delete_media_with_operator(&delete_operator, "_", &delete_origin, &delete_id).await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !deletion.is_finished(),
            "deletion bypassed the publisher's database fence"
        );
        release_tx.send(()).unwrap();
        let thumbnail_key = publisher.await.unwrap().unwrap();
        deletion.await.unwrap().unwrap();
        assert!(!operator.exists(&original_key).await.unwrap());
        assert!(!operator.exists(&thumbnail_key).await.unwrap());
        assert!(
            data::media::get_metadata(&origin, &media_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            data::media::get_thumbnail_by_dimension(&origin, &media_id, 96, 96)
                .await
                .unwrap()
                .is_none()
        );
    }
}
