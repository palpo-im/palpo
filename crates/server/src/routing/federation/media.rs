use std::io::Cursor;
use std::str::FromStr;

use diesel::prelude::*;
use image::imageops::FilterType;
use mime::Mime;
use palpo_core::http_headers::ContentDispositionType;
use salvo::prelude::*;

use crate::core::UnixMillis;
use crate::core::federation::media::*;
use crate::data::connect;
use crate::data::media::*;
use crate::data::schema::*;
use crate::media::{media_storage_key, thumbnail_storage_key};
use crate::utils::content_disposition::make_content_disposition;
use crate::{AppResult, AuthArgs, MatrixError, config, hoops, storage};

pub fn router() -> Router {
    Router::with_path("media")
        .hoop(hoops::limit_rate)
        .push(Router::with_path("download/{media_id}").get(get_content))
        .push(Router::with_path("thumbnail/{media_id}").get(get_thumbnail))
}

/// #GET /_matrix/media/r0/download/{server_name}/{media_id}
/// Load media from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
#[endpoint]
pub async fn get_content(
    args: ContentReqArgs,
    _req: &mut Request,
    res: &mut Response,
) -> AppResult<()> {
    let server_name = &config::get().server_name;
    if let Some(metadata) = crate::data::media::get_metadata(server_name, &args.media_id)? {
        let content_type = metadata
            .content_type
            .as_deref()
            .and_then(|c| Mime::from_str(c).ok())
            .unwrap_or_else(|| {
                metadata
                    .file_name
                    .as_ref()
                    .map(|name| mime_infer::from_path(name).first_or_octet_stream())
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM)
            });

        let key = media_storage_key(server_name, &args.media_id);
        if storage::exists(&key).await? {
            // Try presigned URL redirect for S3 storage
            if let Some(url) = storage::presign_read(&key).await? {
                res.render(salvo::prelude::Redirect::found(url));
                return Ok(());
            }
            let data = storage::read(&key).await?;
            res.add_header("Content-Type", content_type.to_string(), true)?;
            res.body = salvo::http::ResBody::Once(data.into());
            Ok(())
        } else {
            Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into())
        }
    } else {
        Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into())
    }
}

/// # `GET /_matrix/federation/v1/media/thumbnail/{serverName}/{mediaId}`
#[endpoint]
pub async fn get_thumbnail(
    _aa: AuthArgs,
    args: ThumbnailReqArgs,
    _req: &mut Request,
    res: &mut Response,
) -> AppResult<()> {
    let server_name = &config::get().server_name;
    if let Some(DbThumbnail {
        id, content_type, ..
    }) = crate::data::media::get_thumbnail_by_dimension(
        server_name,
        &args.media_id,
        args.width,
        args.height,
    )? {
        let key = thumbnail_storage_key(server_name, &args.media_id, id);
        // Try presigned URL redirect for S3 storage
        if let Some(url) = storage::presign_read(&key).await? {
            res.render(ThumbnailResBody {
                content: FileOrLocation::Location(url),
                metadata: ContentMetadata::new(),
            });
            return Ok(());
        }
        let file_data = storage::read(&key).await?;

        let content_disposition = make_content_disposition(
            Some(ContentDispositionType::Inline),
            content_type.as_deref(),
            None,
        );
        let content = Content {
            file: file_data,
            content_type,
            content_disposition: Some(content_disposition),
        };

        res.render(ThumbnailResBody {
            content: FileOrLocation::File(content),
            metadata: ContentMetadata::new(),
        });
        return Ok(());
    }

    let (width, height, crop) =
        crate::media::thumbnail_properties(args.width, args.height).unwrap_or((0, 0, false)); // 0, 0 because that's the original file

    if let Some(DbThumbnail {
        id, content_type, ..
    }) =
        crate::data::media::get_thumbnail_by_dimension(server_name, &args.media_id, width, height)?
    {
        // Using saved thumbnail
        let key = thumbnail_storage_key(server_name, &args.media_id, id);
        // Try presigned URL redirect for S3 storage
        if let Some(url) = storage::presign_read(&key).await? {
            res.render(ThumbnailResBody {
                content: FileOrLocation::Location(url),
                metadata: ContentMetadata::new(),
            });
            return Ok(());
        }
        let file_data = storage::read(&key).await?;
        let content_disposition = make_content_disposition(
            Some(ContentDispositionType::Inline),
            content_type.as_deref(),
            None,
        );
        let content = Content {
            file: file_data,
            content_type,
            content_disposition: Some(content_disposition),
        };

        res.render(ThumbnailResBody {
            content: FileOrLocation::File(content),
            metadata: ContentMetadata::new(),
        });
        Ok(())
    } else if let Ok(Some(DbMetadata {
        disposition_type: _,
        content_type,
        ..
    })) = crate::data::media::get_metadata(server_name, &args.media_id)
    {
        // Generate a thumbnail: read original from storage
        let image_key = media_storage_key(server_name, &args.media_id);
        let image_data = storage::read(&image_key).await?;

        if let Ok(image) = image::load_from_memory(&image_data) {
            let original_width = image.width();
            let original_height = image.height();
            if width > original_width || height > original_height {
                let content_disposition = make_content_disposition(
                    Some(ContentDispositionType::Inline),
                    content_type.as_deref(),
                    None,
                );
                let content = Content {
                    file: image_data,
                    content_type,
                    content_disposition: Some(content_disposition),
                };

                res.render(ThumbnailResBody {
                    content: FileOrLocation::File(content),
                    metadata: ContentMetadata::new(),
                });
                return Ok(());
            }

            let thumbnail = if crop {
                image.resize_to_fill(width, height, FilterType::CatmullRom)
            } else {
                let (exact_width, exact_height) = {
                    let ratio = u64::from(original_width) * u64::from(height);
                    let nratio = u64::from(width) * u64::from(original_height);

                    let use_width = nratio <= ratio;
                    let intermediate = if use_width {
                        u64::from(original_height) * u64::from(width) / u64::from(original_width)
                    } else {
                        u64::from(original_width) * u64::from(height) / u64::from(original_height)
                    };
                    if use_width {
                        if intermediate <= u64::from(u32::MAX) {
                            (width, intermediate as u32)
                        } else {
                            (
                                (u64::from(width) * u64::from(u32::MAX) / intermediate) as u32,
                                u32::MAX,
                            )
                        }
                    } else if intermediate <= u64::from(u32::MAX) {
                        (intermediate as u32, height)
                    } else {
                        (
                            u32::MAX,
                            (u64::from(height) * u64::from(u32::MAX) / intermediate) as u32,
                        )
                    }
                };

                image.thumbnail_exact(exact_width, exact_height)
            };

            let mut thumbnail_bytes = Vec::new();
            thumbnail.write_to(
                &mut Cursor::new(&mut thumbnail_bytes),
                image::ImageFormat::Png,
            )?;

            // Save thumbnail in database so we don't have to generate it again next time
            diesel::insert_into(media_thumbnails::table)
                .values(&NewDbThumbnail {
                    media_id: args.media_id.clone(),
                    origin_server: server_name.to_owned(),
                    content_type: Some("image/png".to_owned()),
                    disposition_type: None,
                    file_size: thumbnail_bytes.len() as i64,
                    width: width as i32,
                    height: height as i32,
                    resize_method: args.method.clone().unwrap_or_default().to_string(),
                    created_at: UnixMillis::now(),
                })
                .execute(&mut connect()?)?;

            // Save to storage backend
            let thumb_key =
                media_storage_key(server_name, &format!("{}.{width}x{height}", &args.media_id));
            storage::write(&thumb_key, &thumbnail_bytes).await?;

            let content_disposition = make_content_disposition(
                Some(ContentDispositionType::Inline),
                content_type.as_deref(),
                None,
            );
            let content = Content {
                file: thumbnail_bytes,
                content_type,
                content_disposition: Some(content_disposition),
            };

            res.render(ThumbnailResBody {
                content: FileOrLocation::File(content),
                metadata: ContentMetadata::new(),
            });
            Ok(())
        } else {
            let content_disposition = make_content_disposition(None, content_type.as_deref(), None);
            let content = Content {
                file: image_data,
                content_type,
                content_disposition: Some(content_disposition),
            };

            res.render(ThumbnailResBody {
                content: FileOrLocation::File(content),
                metadata: ContentMetadata::new(),
            });
            Ok(())
        }
    } else {
        Err(MatrixError::not_found("file not found").into())
    }
}
