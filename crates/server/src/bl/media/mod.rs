mod metadata;
mod thumbnail;

pub use metadata::*;
pub use thumbnail::*;

use std::time::Duration;

use salvo::Response;

use crate::core::client::media::ContentReqArgs;
use crate::core::ServerName;
use crate::{exts::*, join_path, AppResult};

pub async fn get_remote_content(
    mxc: &str,
    server_name: &ServerName,
    media_id: &str,
    res: &mut Response,
) -> AppResult<()> {
    // let servername: crate::core::OwnedServerName = "127.0.0.1:8448".parse().unwrap();
    // let content_response = match crate::sending::get(servername.build_url(&format!("media/{media_id}?allow_remote=false"))?)
    //     .exec()
    //     .await {
    //         Ok(s) => s,
    //         Err(e) => {
    //             return Err(e.into());
    //         }
    //     };
    let mut content_req = crate::core::client::media::content_request(
        &server_name.origin().await,
        ContentReqArgs {
            server_name: server_name.to_owned(),
            media_id: media_id.to_owned(),
            allow_remote: false,
            timeout_ms: Duration::from_secs(20),
            allow_redirect: false,
        },
    )?
    .into_inner();
    let content_response: reqwest::Response = crate::sending::send_federation_request(server_name, content_req).await?;

    *res.headers_mut() = content_response.headers().to_owned();
    res.status_code(content_response.status());
    res.stream(content_response.bytes_stream());

    // crate::media::create_media(
    //     mxc.to_owned(),
    //     content_response.content_disposition.as_deref(),
    //     content_response.content_type.as_deref(),
    //     &content_response.file,
    // )
    // .await?;

    Ok(())
}

fn get_media_path(key: &str) -> String {
    join_path!(&crate::config().space_path, "media", key)
}
