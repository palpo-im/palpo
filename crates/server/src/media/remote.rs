use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::Response;
use salvo::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use salvo::http::{ResBody, StatusCode};

use super::{Dimension, FileMeta};
use crate::core::federation::media::{ContentReqArgs, FileOrLocation, try_from_multipart_mixed};
use crate::core::http_headers::ContentDisposition;
use crate::core::identifiers::*;
use crate::core::{Mxc, ServerName, UserId};
use crate::data::connect;
use crate::data::schema::*;
use crate::exts::*;
use crate::utils::content_disposition::make_content_disposition;
use crate::utils::read_response_limited;
use crate::{AppError, AppResult, config};

pub async fn fetch_remote_content(
    server_name: &ServerName,
    media_id: &str,
    res: &mut Response,
) -> AppResult<()> {
    check_fetch_authorized(&Mxc {
        server_name,
        media_id,
    })?;

    let content_req = crate::core::media::content_request(
        &server_name.origin().await,
        crate::core::media::ContentReqArgs {
            server_name: server_name.to_owned(),
            media_id: media_id.to_owned(),
            timeout_ms: Duration::from_secs(20),
            allow_remote: true,
            allow_redirect: true,
        },
    )?
    .into_inner();
    let content_response = if let Ok(content_response) =
        crate::sending::send_federation_request(server_name, content_req, None).await
    {
        content_response
    } else {
        let content_req = crate::core::federation::media::content_request(
            &server_name.origin().await,
            ContentReqArgs {
                media_id: media_id.to_owned(),
                timeout_ms: Duration::from_secs(20),
            },
        )?
        .into_inner();
        crate::sending::send_federation_request(server_name, content_req, None).await?
    };

    let Some(content_type_header) = response_content_type(&content_response)
        .filter(|content_type| is_multipart_mixed(content_type))
    else {
        // The legacy endpoint answers with the file itself, so it can be
        // forwarded as it is streamed in.
        for (key, value) in content_response.headers().iter() {
            res.headers_mut().insert(key.clone(), value.clone());
        }
        res.status_code(content_response.status());
        res.stream(content_response.bytes_stream());
        return Ok(());
    };

    let body =
        read_response_limited(content_response, config::get().media.max_remote_media_size).await?;

    let content = match try_from_multipart_mixed(&content_type_header, &body) {
        Ok((_metadata, FileOrLocation::File(content))) => content,
        Ok((_metadata, FileOrLocation::Location(location))) => {
            // Following the location requires an outbound request to a server
            // controlled URL, which needs its own protections before we can
            // make it.
            warn!("remote media {media_id} on {server_name} is served from {location}");
            render_bad_gateway(
                res,
                "Remote media is served from an external location, which is not supported",
            );
            return Ok(());
        }
        Err(e) => {
            warn!("failed to parse media response from {server_name}: {e}");
            render_bad_gateway(res, "Failed to parse remote media response");
            return Ok(());
        }
    };

    let content_type = content
        .content_type
        .filter(|content_type| !content_type.is_empty())
        .unwrap_or_else(|| mime::APPLICATION_OCTET_STREAM.to_string());
    // Keep the filename the remote server sent, but decide for ourselves
    // whether the file may be displayed inline.
    let content_disposition = make_content_disposition(
        None,
        Some(&content_type),
        content
            .content_disposition
            .as_ref()
            .and_then(|disposition| disposition.filename.as_deref()),
    );

    res.add_header(CONTENT_TYPE, &content_type, true)?;
    res.add_header(CONTENT_DISPOSITION, content_disposition.to_string(), true)?;
    res.add_header("Cross-Origin-Resource-Policy", "cross-origin", true)?;
    res.status_code(StatusCode::OK);
    res.body = ResBody::Once(content.file.into());

    Ok(())
}

/// Read a federation media response, unwrapping the `multipart/mixed` body the
/// authenticated endpoints answer with.
async fn read_media_response(response: reqwest::Response, max_size: usize) -> AppResult<FileMeta> {
    let content_type = response_content_type(&response);

    let Some(content_type) = content_type
        .clone()
        .filter(|content_type| is_multipart_mixed(content_type))
    else {
        let content_disposition = response
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| ContentDisposition::from_str(s).ok());
        let file = read_response_limited(response, max_size).await?.to_vec();

        return Ok(FileMeta {
            content: Some(file),
            content_type,
            content_disposition,
        });
    };

    let body = read_response_limited(response, max_size).await?;
    let (_metadata, content) = try_from_multipart_mixed(&content_type, &body)
        .map_err(|e| AppError::public(format!("Failed to parse remote media response: {e}")))?;

    match content {
        FileOrLocation::File(content) => Ok(FileMeta {
            content: Some(content.file),
            content_type: content.content_type,
            content_disposition: content.content_disposition,
        }),
        FileOrLocation::Location(_) => Err(AppError::public(
            "Remote media is served from an external location, which is not supported",
        )),
    }
}

fn response_content_type(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
}

fn is_multipart_mixed(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .eq_ignore_ascii_case("multipart/mixed")
}

fn render_bad_gateway(res: &mut Response, error: &str) {
    res.status_code(StatusCode::BAD_GATEWAY);
    res.body = ResBody::Once(
        serde_json::json!({ "errcode": "M_UNKNOWN", "error": error })
            .to_string()
            .into(),
    );
}

pub async fn fetch_remote_thumbnail(
    mxc: &Mxc<'_>,
    user: Option<&UserId>,
    server: Option<&ServerName>,
    timeout_ms: Duration,
    dim: &Dimension,
) -> AppResult<FileMeta> {
    check_fetch_authorized(mxc)?;

    let result = fetch_thumbnail_authenticated(mxc, user, server, timeout_ms, dim).await;

    if result.is_err() {
        return fetch_thumbnail_unauthenticated(mxc, user, server, timeout_ms, dim).await;
    }

    result
}

// pub async fn fetch_remote_content(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     server: Option<&ServerName>,
//     timeout_ms: Duration,
// ) -> AppResult<FileMeta> {
//     check_fetch_authorized(mxc)?;

//     let result = fetch_content_authenticated(mxc, user, server, timeout_ms).await;

//     if result.is_err() {
//         return fetch_content_unauthenticated(mxc, user, server, timeout_ms).await;
//     }

//     result
// }

async fn fetch_thumbnail_authenticated(
    mxc: &Mxc<'_>,
    user: Option<&UserId>,
    server: Option<&ServerName>,
    timeout_ms: Duration,
    dim: &Dimension,
) -> AppResult<FileMeta> {
    let target_server = server.unwrap_or(mxc.server_name);
    let origin = target_server.origin().await;

    // Build the federation thumbnail request using the authenticated media API (v1)
    let thumbnail_req = crate::core::federation::media::thumbnail_request(
        &origin,
        crate::core::federation::media::ThumbnailReqArgs {
            media_id: mxc.media_id.to_owned(),
            method: Some(dim.method.clone()),
            width: dim.width,
            height: dim.height,
            timeout_ms,
            animated: Some(true),
        },
    )?
    .into_inner();

    // Send federation request. The authenticated endpoint answers with a
    // `multipart/mixed` body, which `read_media_response` unwraps for us.
    let response =
        crate::sending::send_federation_request(target_server, thumbnail_req, None).await?;
    let meta = read_media_response(response, config::get().media.max_remote_thumbnail_size).await?;

    save_fetched_thumbnail(mxc, user, dim, &meta).await;

    Ok(meta)
}

// async fn fetch_content_authenticated(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     server: Option<&ServerName>,
//     timeout_ms: Duration,
// ) -> AppResult<FileMeta> {
// use federation::authenticated_media::get_content::v1::{Request, Response};

// let request = Request {
// 	media_id: mxc.media_id.into(),
// 	timeout_ms,
// };

// let Response { content, .. } = self
// 	.federation_request(mxc, user, server, request)
// 	.await?;

// match content {
// 	| FileOrLocation::File(content) => self.handle_content_file(mxc, user, content).await,
// 	| FileOrLocation::Location(location) => self.handle_location(mxc, user, &location).await,
// }
// }

async fn fetch_thumbnail_unauthenticated(
    mxc: &Mxc<'_>,
    user: Option<&UserId>,
    server: Option<&ServerName>,
    timeout_ms: Duration,
    dim: &Dimension,
) -> AppResult<FileMeta> {
    let target_server = server.unwrap_or(mxc.server_name);
    let origin = target_server.origin().await;

    // Build the legacy (unauthenticated) media thumbnail request (v3)
    let thumbnail_req = crate::core::media::thumbnail_request(
        &origin,
        mxc.server_name,
        crate::core::media::ThumbnailReqArgs {
            server_name: mxc.server_name.to_owned(),
            media_id: mxc.media_id.to_owned(),
            method: Some(dim.method.clone()),
            width: dim.width,
            height: dim.height,
            allow_remote: true,
            timeout_ms,
            allow_redirect: true,
        },
    )?
    .into_inner();

    // Send federation request
    let response =
        crate::sending::send_federation_request(target_server, thumbnail_req, None).await?;
    let meta = read_media_response(response, config::get().media.max_remote_thumbnail_size).await?;

    save_fetched_thumbnail(mxc, user, dim, &meta).await;

    Ok(meta)
}

/// Save a thumbnail fetched from a remote server locally, for caching.
async fn save_fetched_thumbnail(
    mxc: &Mxc<'_>,
    user: Option<&UserId>,
    dim: &Dimension,
    meta: &FileMeta,
) {
    let Some(file) = meta.content.as_deref().filter(|file| !file.is_empty()) else {
        return;
    };

    if let Err(e) = crate::media::save_thumbnail(
        mxc,
        user,
        meta.content_type.as_deref(),
        meta.content_disposition.as_ref(),
        dim,
        file,
    )
    .await
    {
        warn!("Failed to save fetched thumbnail locally: {e}");
    }
}

// async fn fetch_content_unauthenticated(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     server: Option<&ServerName>,
//     timeout_ms: Duration,
// ) -> AppResult<FileMeta> {
// use media::get_content::v3::{Request, Response};

// let request = Request {
// 	allow_remote: true,
// 	allow_redirect: true,
// 	server_name: mxc.server_name.into(),
// 	media_id: mxc.media_id.into(),
// 	timeout_ms,
// };

// let Response {
// 	file, content_type, content_disposition, ..
// } = self
// 	.federation_request(mxc, user, server, request)
// 	.await?;

// let content = Content { file, content_type, content_disposition };

// handle_content_file(mxc, user, content).await
// }

// async fn handle_thumbnail_file(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     dim: &Dimension,
//     content: Content,
// ) -> AppResult<FileMeta> {
//     let content_disposition = make_content_disposition(
//         content.content_disposition,
//         content.content_type.as_deref(),
//         None,
//     );

//     crate::media::save_thumbnail(
//         mxc,
//         user,
//         content.content_type.as_deref(),
//         Some(&content_disposition),
//         dim,
//         &content.file,
//     )
//     .await
//     .map(|()| FileMeta {
//         content: Some(content.file),
//         content_type: content.content_type.map(Into::into),
//         content_disposition: Some(content_disposition),
//     })
// }

// async fn handle_content_file(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     content: Content,
// ) -> AppResult<FileMeta> {
// let content_disposition = make_content_disposition(
// 	content.content_disposition.as_ref(),
// 	content.content_type.as_deref(),
// 	None,
// );

// create(
// 	mxc,
// 	user,
// 	Some(&content_disposition),
// 	content.content_type.as_deref(),
// 	&content.file,
// )
// .await
// .map(|()| FileMeta {
// 	content: Some(content.file),
// 	content_type: content.content_type.map(Into::into),
// 	content_disposition: Some(content_disposition),
// })
// }

// async fn handle_location(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     location: &str,
// ) -> AppResult<FileMeta> {
//     location_request(location)
//         .await
//         .map_err(|error| AppError::public("fetching media from location failed"))
// }

// async fn location_request(location: &str) -> AppResult<FileMeta> {
// let response = self
// 	.services
// 	.client
// 	.extern_media
// 	.get(location)
// 	.send()
// 	.await?;

// let content_type = response
// 	.headers()
// 	.get(CONTENT_TYPE)
// 	.map(HeaderValue::to_str)
// 	.and_then(Result::ok)
// 	.map(str::to_owned);

// let content_disposition = response
// 	.headers()
// 	.get(CONTENT_DISPOSITION)
// 	.map(HeaderValue::as_bytes)
// 	.map(TryFrom::try_from)
// 	.and_then(Result::ok);

// response
// 	.bytes()
// 	.await
// 	.map(Vec::from)
// 	.map_err(Into::into)
// 	.map(|content| FileMeta {
// 		content: Some(content),
// 		content_type: content_type.clone(),
// 		content_disposition: Some(make_content_disposition(
// 			content_disposition.as_ref(),
// 			content_type.as_deref(),
// 			None,
// 		)),
// 	})
// }

// async fn federation_request<Request>(
//     mxc: &Mxc<'_>,
//     user: Option<&UserId>,
//     server: Option<&ServerName>,
//     request: Request,
// ) -> Result<Request::IncomingResponse>
// where
//     Request: OutgoingRequest + Send + Debug,
// {
//     unimplemented!()
// self.services
// 	.sending
// 	.send_federation_request(server.unwrap_or(mxc.server_name), request)
// 	.await
// }

// pub async fn fetch_remote_thumbnail_legacy(
//     body: &media::get_content_thumbnail::v3::Request,
// ) -> AppResult<media::get_content_thumbnail::v3::Response> {
//     unimplemented!()
// let mxc = Mxc {
// 	server_name: &body.server_name,
// 	media_id: &body.media_id,
// };

// self.check_legacy_freeze()?;
// self.check_fetch_authorized(&mxc)?;
// let response = self
// 	.services
// 	.sending
// 	.send_federation_request(mxc.server_name, media::get_content_thumbnail::v3::Request {
// 		allow_remote: body.allow_remote,
// 		height: body.height,
// 		width: body.width,
// 		method: body.method.clone(),
// 		server_name: body.server_name.clone(),
// 		media_id: body.media_id.clone(),
// 		timeout_ms: body.timeout_ms,
// 		allow_redirect: body.allow_redirect,
// 		animated: body.animated,
// 	})
// 	.await?;

// let dim = Dim::from_ruma(body.width, body.height, body.method.clone())?;
// self.upload_thumbnail(
// 	&mxc,
// 	None,
// 	None,
// 	response.content_type.as_deref(),
// 	&dim,
// 	&response.file,
// )
// .await?;

// Ok(response)
// }

// pub async fn fetch_remote_content_legacy(
//     mxc: &Mxc<'_>,
//     allow_redirect: bool,
//     timeout_ms: Duration,
// ) -> AppResult<media::get_content::v3::Response> {
//     unimplemented!()
// self.check_legacy_freeze()?;
// self.check_fetch_authorized(mxc)?;
// let response = self
// 	.services
// 	.sending
// 	.send_federation_request(mxc.server_name, media::get_content::v3::Request {
// 		allow_remote: true,
// 		server_name: mxc.server_name.into(),
// 		media_id: mxc.media_id.into(),
// 		timeout_ms,
// 		allow_redirect,
// 	})
// 	.await?;

// let content_disposition = make_content_disposition(
// 	response.content_disposition.as_ref(),
// 	response.content_type.as_deref(),
// 	None,
// );

// create(
// 	mxc,
// 	None,
// 	Some(&content_disposition),
// 	response.content_type.as_deref(),
// 	&response.file,
// )
// .await?;

// Ok(response)
// }

fn check_fetch_authorized(mxc: &Mxc<'_>) -> AppResult<()> {
    let conf = config::get();
    if conf
        .media
        .prevent_downloads_from
        .is_match(mxc.server_name.host())
        || conf
            .forbidden_remote_server_names
            .is_match(mxc.server_name.host())
    {
        // we'll lie to the client and say the blocked server's media was not found and
        // log. the client has no way of telling anyways so this is a security bonus.
        warn!(%mxc, "Received request for media on blocklisted server");
        return Err(AppError::public("Media not found."));
    }

    Ok(())
}

// fn check_legacy_freeze() -> AppResult<()> {
//     unimplemented!()
// self.services
// 	.server
// 	.config
// 	.freeze_legacy_media
// 	.then_some(())
// 	.ok_or(err!(Request(NotFound("Remote media is frozen."))))
// }

pub async fn delete_past_remote_media(
    time: SystemTime,
    before: bool,
    after: bool,
    yes_i_want_to_delete_local_media: bool,
) -> AppResult<u64> {
    if before && after {
        return Err(AppError::public(
            "Please only pick one argument, --before or --after.",
        ));
    }
    if !(before || after) {
        return Err(AppError::public(
            "Please pick one argument, --before or --after.",
        ));
    }

    let time = time.duration_since(UNIX_EPOCH)?.as_millis();

    let mxcs = if after {
        media_metadatas::table
            .filter(media_metadatas::origin_server.ne(config::server_name()))
            .filter(media_metadatas::created_at.lt(time as i64))
            .select((media_metadatas::origin_server, media_metadatas::media_id))
            .load::<(OwnedServerName, String)>(&mut connect().await?)
            .await?
    } else {
        media_metadatas::table
            .filter(media_metadatas::origin_server.eq(config::server_name()))
            .filter(media_metadatas::created_at.gt(time as i64))
            .select((media_metadatas::origin_server, media_metadatas::media_id))
            .load::<(OwnedServerName, String)>(&mut connect().await?)
            .await?
    };
    let mut count = 0;
    for (origin_server, media_id) in &mxcs {
        let mxc = OwnedMxcUri::from(format!("mxc://{origin_server}/{media_id}"));
        if let Err(e) =
            delete_remote_media(origin_server, media_id, yes_i_want_to_delete_local_media).await
        {
            warn!("failed to delete remote media {mxc}: {e}");
        } else {
            count += 1;
        }
    }
    Ok(count)
}

pub async fn delete_remote_media(
    server_name: &ServerName,
    media_id: &str,
    yes_i_want_to_delete_local_media: bool,
) -> AppResult<()> {
    crate::data::media::delete_media(server_name, media_id).await?;

    if !yes_i_want_to_delete_local_media {
        return Ok(());
    }

    let key = crate::media::media_storage_key(server_name, media_id);
    if let Err(e) = crate::storage::delete(&key).await {
        warn!("failed to delete media file '{key}': {e}");
    }

    Ok(())
}
