use std::io::Cursor;
use std::str::FromStr;

use diesel::prelude::*;
use image::imageops::FilterType;
use mime::Mime;
use reqwest::Url;
use salvo::http::header::CONTENT_TYPE;
use salvo::http::{HeaderValue, ResBody};
use salvo::prelude::*;
use uuid::Uuid;

use crate::core::UnixMillis;
use crate::core::client::media::*;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::media::{DbMetadata, DbThumbnail, NewDbMetadata, NewDbThumbnail};
use crate::data::schema::*;
use crate::exts::*;
use crate::media::*;
use crate::{
    AppResult, AuthArgs, EmptyResult, JsonResult, MatrixError, config, empty_ok, hoops, json_ok,
    storage, utils,
};

pub fn self_auth_router() -> Router {
    Router::with_path("media")
        .oapi_tag("client")
        .push(
            Router::with_path("download/{server_name}/{media_id}")
                .hoop(hoops::auth_by_access_token_or_signatures)
                .get(get_content)
                .push(Router::with_path("{filename}").get(get_content_with_filename)),
        )
        .push(
            Router::with_hoop(hoops::limit_rate)
                .hoop(hoops::auth_by_access_token)
                .push(Router::with_path("config").get(get_config))
                .push(Router::with_path("preview_url").get(preview_url))
                .push(Router::with_path("thumbnail/{server_name}/{media_id}").get(get_thumbnail)),
        )
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
    if let Some(metadata) = crate::data::media::get_metadata(&args.server_name, &args.media_id)? {
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

        let key = media_storage_key(&args.server_name, &args.media_id);
        if storage::exists(&key).await? {
            let data = storage::read(&key).await?;
            res.add_header(CONTENT_TYPE, content_type.to_string(), true)?;
            if let Some(file_name) = &metadata.file_name {
                res.add_header(
                    "Content-Disposition",
                    format!(r#"attachment; filename="{file_name}""#),
                    true,
                )?;
            }
            res.body = ResBody::Once(data.into());
            Ok(())
        } else {
            Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into())
        }
    } else if *args.server_name != config::get().server_name && args.allow_remote {
        let mxc = format!("mxc://{}/{}", args.server_name, args.media_id);
        fetch_remote_content(&mxc, &args.server_name, &args.media_id, res).await
    } else {
        Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into())
    }
}

/// #GET /_matrix/media/r0/download/{server_name}/{media_id}/{file_name}
/// Load media from our server or over federation, permitting desired filename.
///
/// - Only allows federation if `allow_remote` is true
#[endpoint]
pub async fn get_content_with_filename(
    args: ContentWithFileNameReqArgs,
    _req: &mut Request,
    res: &mut Response,
) -> AppResult<()> {
    let Some(metadata) = crate::data::media::get_metadata(&args.server_name, &args.media_id)?
    else {
        return Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into());
    };
    let content_type = if let Some(content_type) = metadata.content_type.as_deref() {
        content_type.to_owned()
    } else {
        metadata
            .file_name
            .as_ref()
            .map(|name| mime_infer::from_path(name).first_or_octet_stream())
            .unwrap_or(mime::APPLICATION_OCTET_STREAM)
            .to_string()
    };
    if let Ok(content_type) = content_type.parse::<HeaderValue>() {
        res.headers_mut().insert(CONTENT_TYPE, content_type);
    }

    let key = media_storage_key(&args.server_name, &args.media_id);
    if storage::exists(&key).await? {
        let data = storage::read(&key).await?;
        res.add_header(
            "Content-Disposition",
            format!(r#"attachment; filename="{}""#, args.filename),
            true,
        )?;
        res.body = ResBody::Once(data.into());
        Ok(())
    } else if *args.server_name != config::get().server_name && args.allow_remote {
        let mxc = format!("mxc://{}/{}", args.server_name, args.media_id);
        fetch_remote_content(&mxc, &args.server_name, &args.media_id, res).await
    } else {
        Err(MatrixError::not_yet_uploaded("Media has not been uploaded yet").into())
    }
}
#[endpoint]
pub fn create_mxc_uri(_aa: AuthArgs) -> JsonResult<CreateMxcUriResBody> {
    let media_id = utils::random_string(crate::MXC_LENGTH);
    let mxc = format!("mxc://{}/{}", config::get().server_name, media_id);
    Ok(Json(CreateMxcUriResBody {
        content_uri: OwnedMxcUri::from(mxc),
        unused_expires_at: None,
    }))
}

/// #POST /_matrix/media/r0/upload
/// Permanently save media in the server.
///
/// - Some metadata will be saved in the database
/// - Media will be saved via the configured storage backend
#[endpoint]
pub async fn create_content(
    _aa: AuthArgs,
    args: CreateContentReqArgs,
    req: &mut Request,
    _depot: &mut Depot,
) -> JsonResult<CreateContentResBody> {
    let file_name = args.filename.clone();
    let file_extension = file_name.as_deref().map(utils::fs::get_file_ext);

    let payload = req
        .payload_with_max_size(config::get().max_upload_size as usize)
        .await
        .map_err(|e| MatrixError::too_large(format!("Failed to read upload payload: {e}")))?;

    let media_id = utils::base32_crockford(Uuid::new_v4().as_bytes());
    let mxc = Mxc {
        server_name: &config::get().server_name,
        media_id: &media_id,
    };

    let conf = crate::config::get();
    let key = media_storage_key(&conf.server_name, &media_id);

    if !storage::exists(&key).await? {
        storage::write(&key, payload).await?;

        let metadata = NewDbMetadata {
            media_id: media_id.clone(),
            origin_server: conf.server_name.clone(),
            disposition_type: Some("inline".into()),
            content_type: args.content_type.clone(),
            file_name,
            file_extension,
            file_size: payload.len() as i64,
            file_hash: None,
            created_by: None,
            created_at: UnixMillis::now(),
        };

        crate::data::media::insert_metadata(&metadata)?;
    } else {
        return Err(MatrixError::cannot_overwrite_media("Media ID already has content").into());
    }

    json_ok(CreateContentResBody {
        content_uri: mxc.to_string().into(),
        blurhash: None,
    })
}

/// #PUT /_matrix/media/*/upload/{server_name}/{media_id}
/// Upload media to an MXC URI that was created with create_mxc_uri.
#[endpoint]
pub async fn upload_content(
    _aa: AuthArgs,
    args: UploadContentReqArgs,
    req: &mut Request,
    _depot: &mut Depot,
) -> EmptyResult {
    let file_name = args.filename.clone();
    let file_extension = file_name.as_deref().map(utils::fs::get_file_ext);

    let conf = crate::config::get();
    let payload = req
        .payload_with_max_size(conf.max_upload_size as usize)
        .await
        .map_err(|e| MatrixError::too_large(format!("Failed to read upload payload: {e}")))?;

    let key = media_storage_key(&conf.server_name, &args.media_id);

    if !storage::exists(&key).await? {
        storage::write(&key, payload).await?;

        let metadata = NewDbMetadata {
            media_id: args.media_id.clone(),
            origin_server: conf.server_name.clone(),
            disposition_type: args
                .filename
                .clone()
                .map(|filename| format!(r#"inline; filename="{filename}""#)),
            content_type: args.content_type.clone(),
            file_name,
            file_extension,
            file_size: payload.len() as i64,
            file_hash: None,
            created_by: None,
            created_at: UnixMillis::now(),
        };

        crate::data::media::insert_metadata(&metadata)?;

        empty_ok()
    } else {
        Err(MatrixError::cannot_overwrite_media("Media ID already has content").into())
    }
}

/// #GET /_matrix/media/r0/config
/// Returns max upload size.
#[endpoint]
pub async fn get_config(_aa: AuthArgs) -> JsonResult<ConfigResBody> {
    json_ok(ConfigResBody {
        upload_size: config::get().max_upload_size.into(),
    })
}

/// # `GET /_matrix/client/v1/media/preview_url`
///
/// Returns URL preview.
#[endpoint]
pub async fn preview_url(
    _aa: AuthArgs,
    args: MediaPreviewReqArgs,
    depot: &mut Depot,
) -> JsonResult<MediaPreviewResBody> {
    let _sender_id = depot.authed_info()?.user_id();

    let url = Url::parse(&args.url)
        .map_err(|e| MatrixError::invalid_param(format!("Requested URL is not valid: {e}")))?;

    if !crate::media::url_preview_allowed(&url) {
        return Err(MatrixError::forbidden("URL is not allowed to be previewed", None).into());
    }

    let preview = crate::media::get_url_preview(&url).await?;

    let res_body = MediaPreviewResBody::from_serialize(&preview)
        .map_err(|e| MatrixError::unknown(format!("Failed to parse URL preview: {e}")))?;
    json_ok(res_body)
}

//// #GET /_matrix/media/r0/thumbnail/{server_name}/{media_id}
/// Load media thumbnail from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// Downloads a file's thumbnail.
///
/// Here's an example on how it works:
///
/// - Client requests an image with width=567, height=567
/// - Server rounds that up to (800, 600), so it doesn't have to save too many thumbnails
/// - Server rounds that up again to (958, 600) to fix the aspect ratio (only for width,height>96)
/// - Server creates the thumbnail and sends it to the user
///
/// For width,height <= 96 the server uses another thumbnailing algorithm which crops the image
/// afterwards.
#[endpoint]
pub async fn get_thumbnail(
    _aa: AuthArgs,
    args: ThumbnailReqArgs,
    _req: &mut Request,
    res: &mut Response,
) -> AppResult<()> {
    if args.server_name.is_remote() && args.allow_remote {
        let origin = args.server_name.origin().await;
        let mut url = Url::parse(&format!(
            "{}/_matrix/media/v3/thumbnail/{}/{}",
            origin, args.server_name, args.media_id
        ))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("width", &args.width.to_string());
            query.append_pair("height", &args.height.to_string());
            query.append_pair("timeout_ms", &args.timeout_ms.as_millis().to_string());
        }
        let request = crate::sending::get(url).into_inner();
        let response =
            crate::sending::send_federation_request(&args.server_name, request, None).await?;
        *res.headers_mut() = response.headers().clone();
        let bytes = response.bytes().await?;

        // Cache the remote thumbnail via storage backend
        let cache_key = media_storage_key(
            &args.server_name,
            &format!("{}.{}x{}", args.media_id, args.width, args.height),
        );
        if let Err(e) = storage::write(&cache_key, &bytes).await {
            tracing::warn!("Failed to cache remote thumbnail: {e}");
        }

        res.body = ResBody::Once(bytes);
        return Ok(());
    }

    match crate::data::media::get_thumbnail_by_dimension(
        &args.server_name,
        &args.media_id,
        args.width,
        args.height,
    ) {
        Ok(Some(DbThumbnail {
            id,
            content_type,
            ..
        })) => {
            let key = thumbnail_storage_key(&args.server_name, &args.media_id, id);
            let data = storage::read(&key).await?;
            let ct = content_type.as_deref().unwrap_or("application/octet-stream");
            res.add_header("Cross-Origin-Resource-Policy", "cross-origin", true)?;
            res.add_header(CONTENT_TYPE, ct, true)?;
            res.body = ResBody::Once(data.into());
            return Ok(());
        }
        Err(e) => {
            tracing::error!(error = ?e, "get_thumbnail error");
            return Err(MatrixError::not_found("Media not found.").into());
        }
        _ => {}
    }

    let (width, height, crop) =
        crate::media::thumbnail_properties(args.width, args.height).unwrap_or((0, 0, false)); // 0, 0 because that's the original file

    if let Some(DbThumbnail {
        id, content_type, ..
    }) = crate::data::media::get_thumbnail_by_dimension(
        &args.server_name,
        &args.media_id,
        width,
        height,
    )? {
        let key = thumbnail_storage_key(&args.server_name, &args.media_id, id);
        let data = storage::read(&key).await?;
        let ct = content_type.as_deref().unwrap_or("application/octet-stream");
        res.add_header(CONTENT_TYPE, ct, true)?;
        res.body = ResBody::Once(data.into());
        Ok(())
    } else if let Ok(Some(DbMetadata {
        disposition_type: _,
        content_type,
        ..
    })) = crate::data::media::get_metadata(&args.server_name, &args.media_id)
    {
        // Generate a thumbnail: read original image from storage
        let image_key = media_storage_key(&args.server_name, &args.media_id);
        let image_data = storage::read(&image_key).await?;
        let ct = content_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        if let Ok(image) = image::load_from_memory(&image_data) {
            let original_width = image.width();
            let original_height = image.height();
            if width > original_width || height > original_height {
                res.add_header(CONTENT_TYPE, ct, true)?;
                res.body = ResBody::Once(image_data.into());
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
            let thumbnail_id = diesel::insert_into(media_thumbnails::table)
                .values(&NewDbThumbnail {
                    media_id: args.media_id.clone(),
                    origin_server: args.server_name.clone(),
                    content_type: Some("image/png".to_owned()),
                    disposition_type: None,
                    file_size: thumbnail_bytes.len() as i64,
                    width: width as i32,
                    height: height as i32,
                    resize_method: args.method.clone().unwrap_or_default().to_string(),
                    created_at: UnixMillis::now(),
                })
                .on_conflict_do_nothing()
                .returning(media_thumbnails::id)
                .get_result::<i64>(&mut connect()?)
                .optional()?;
            let thumbnail_id = if let Some(thumbnail_id) = thumbnail_id {
                crate::media::save_thumbnail_file(
                    &args.server_name,
                    &args.media_id,
                    thumbnail_id,
                    &thumbnail_bytes,
                )
                .await?;
                thumbnail_id
            } else {
                media_thumbnails::table
                    .filter(media_thumbnails::media_id.eq(&args.media_id))
                    .filter(media_thumbnails::width.eq(args.width as i32))
                    .filter(media_thumbnails::height.eq(args.height as i32))
                    .filter(
                        media_thumbnails::resize_method.eq(&args
                            .method
                            .clone()
                            .unwrap_or_default()
                            .to_string()),
                    )
                    .select(media_thumbnails::id)
                    .first::<i64>(&mut connect()?)?
            };

            // Return the newly generated thumbnail
            let key = thumbnail_storage_key(&args.server_name, &args.media_id, thumbnail_id);
            let data = storage::read(&key).await?;
            res.add_header(CONTENT_TYPE, "image/png", true)?;
            res.body = ResBody::Once(data.into());
            Ok(())
        } else {
            // Couldn't parse file to generate thumbnail, send original
            res.add_header(CONTENT_TYPE, ct, true)?;
            res.body = ResBody::Once(image_data.into());
            Ok(())
        }
    } else {
        Err(MatrixError::not_found("file not found").into())
    }
}
