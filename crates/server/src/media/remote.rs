use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diesel::prelude::*;
use salvo::Response;

use super::{Dimension, FileMeta};
use crate::core::federation::media::ContentReqArgs;
use crate::core::identifiers::*;
use crate::core::{Mxc, ServerName, UserId};
use crate::data::connect;
use crate::data::schema::*;
use crate::exts::*;
use crate::{AppError, AppResult, config};

pub async fn fetch_remote_content(
    _mxc: &str,
    server_name: &ServerName,
    media_id: &str,
    res: &mut Response,
) -> AppResult<()> {
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

    *res.headers_mut() = content_response.headers().to_owned();
    res.status_code(content_response.status());
    res.stream(content_response.bytes_stream());

    Ok(())
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
    unimplemented!()
    // use federation::authenticated_media::get_content_thumbnail::v1::{Request, Response};

    // let request = Request {
    // 	media_id: mxc.media_id.into(),
    // 	method: dim.method.clone().into(),
    // 	width: dim.width.into(),
    // 	height: dim.height.into(),
    // 	animated: true.into(),
    // 	timeout_ms,
    // };

    // let Response { content, .. } = self
    // 	.federation_request(mxc, user, server, request)
    // 	.await?;

    // match content {
    // 	| FileOrLocation::File(content) =>
    // 		self.handle_thumbnail_file(mxc, user, dim, content)
    // 			.await,
    // 	| FileOrLocation::Location(location) => self.handle_location(mxc, user, &location).await,
    // }
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
    unimplemented!()
    // use media::get_content_thumbnail::v3::{Request, Response};

    // let request = Request {
    // 	allow_remote: true,
    // 	allow_redirect: true,
    // 	animated: true.into(),
    // 	method: dim.method.clone().into(),
    // 	width: dim.width.into(),
    // 	height: dim.height.into(),
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

    // handle_thumbnail_file(mxc, user, dim, content)
    // 	.await
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
            .load::<(OwnedServerName, String)>(&mut connect()?)?
    } else {
        media_metadatas::table
            .filter(media_metadatas::origin_server.eq(config::server_name()))
            .filter(media_metadatas::created_at.gt(time as i64))
            .select((media_metadatas::origin_server, media_metadatas::media_id))
            .load::<(OwnedServerName, String)>(&mut connect()?)?
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
    crate::data::media::delete_media(server_name, media_id)?;

    if !yes_i_want_to_delete_local_media {
        return Ok(());
    }

    let path = crate::media::get_media_path(server_name, media_id);
    if let Err(e) = std::fs::remove_file(&path) {
        warn!("failed to delete local media file {path:?}: {e}");
    }

    Ok(())
}
