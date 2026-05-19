use salvo::prelude::*;

use super::client::media::*;
use crate::hoops;

pub fn router() -> Router {
    let mut media = Router::with_path("media")
        .oapi_tag("media")
        .hoop(hoops::media_security_headers);
    for v in ["v3", "v1", "r0"] {
        media = media
            .push(
                Router::with_path(v)
                    .hoop(hoops::auth_by_access_token)
                    .push(Router::with_path("create").post(create_mxc_uri))
                    .push(
                        Router::with_path("upload").post(create_content).push(
                            Router::with_path("{server_name}/{media_id}").put(upload_content),
                        ),
                    )
                    .push(
                        Router::with_hoop(hoops::limit_rate)
                            .push(Router::with_path("config").get(get_config))
                            .push(Router::with_path("preview_url").get(preview_url)),
                    ),
            )
            .push(
                Router::with_path(v)
                    .push(
                        Router::with_path("download/{server_name}/{media_id}")
                            .get(get_content)
                            .push(Router::with_path("{filename}").get(get_content_with_filename)),
                    )
                    .push(Router::with_hoop(hoops::limit_rate).push(
                        Router::with_path("thumbnail/{server_name}/{media_id}").get(get_thumbnail),
                    )),
            )
    }
    media
}
