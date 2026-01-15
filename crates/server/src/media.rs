mod preview;
mod remote;
use std::cmp;
use std::num::Saturating;
use std::path::PathBuf;

use diesel::prelude::*;
pub use preview::*;
pub use remote::*;
use tokio::io::AsyncWriteExt;

use crate::core::http_headers::ContentDisposition;
use crate::core::media::ResizeMethod;
use crate::core::{Mxc, OwnedMxcUri, ServerName, UnixMillis, UserId};
use crate::data::connect;
use crate::data::media::NewDbThumbnail;
use crate::data::schema::*;
use crate::{AppResult, config, data};

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

pub fn get_media_path(server_name: &ServerName, media_id: &str) -> PathBuf {
    let server_name = if server_name == config::server_name() {
        "_"
    } else {
        server_name.as_str()
    };
    let mut r = PathBuf::new();
    r.push(config::space_path());
    r.push("media");
    r.push(server_name);
    // let extension = extension.unwrap_or_default();
    // if !extension.is_empty() {
    //     r.push(format!("{media_id}.{extension}"));
    // } else {
    r.push(media_id);
    // }
    r
}

pub async fn delete_media(server_name: &ServerName, media_id: &str) -> AppResult<()> {
    data::media::delete_media(server_name, media_id)?;
    let path = get_media_path(server_name, media_id);
    if path.exists()
        && let Err(e) = tokio::fs::remove_file(path).await
    {
        tracing::error!("failed to delete media file: {e}");
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

pub fn get_all_mxcs() -> AppResult<Vec<OwnedMxcUri>> {
    let mxcs = media_metadatas::table
        .select((media_metadatas::origin_server, media_metadatas::media_id))
        .load::<(String, String)>(&mut connect()?)?
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
    user: Option<&UserId>,
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
        .get_result::<i64>(&mut connect()?)
        .optional()?;

    if let Some(id) = id {
        save_thumbnail_file(mxc.server_name, mxc.media_id, id, file).await?;
    }

    Ok(())
}

pub async fn save_thumbnail_file(
    server_name: &ServerName,
    media_id: &str,
    thumbnail_id: i64,
    file: &[u8],
) -> AppResult<PathBuf> {
    let thumb_path = get_thumbnail_path(server_name, media_id, thumbnail_id);
    let mut f = tokio::fs::File::create(&thumb_path).await?;
    f.write_all(file).await?;
    Ok(thumb_path)
}

pub fn get_thumbnail_path(server_name: &ServerName, media_id: &str, thumbnail_id: i64) -> PathBuf {
    get_media_path(
        server_name,
        &format!("{media_id}.thumbnails/{thumbnail_id}"),
    )
}
