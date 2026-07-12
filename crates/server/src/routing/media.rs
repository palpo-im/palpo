use salvo::prelude::*;

use super::client::media::*;
use crate::config::MediaConfig;
use crate::{config, hoops};

fn legacy_media_enabled(media: &MediaConfig) -> bool {
    media.allow_legacy
}

fn legacy_url_preview_enabled(media: &MediaConfig) -> bool {
    media.allow_legacy && !media.freeze_legacy
}

pub fn router() -> Router {
    let media_config = &config::get().media;
    let mut media = Router::with_path("media").oapi_tag("media");
    if !legacy_media_enabled(media_config) {
        return media;
    }

    for v in ["v3", "v1", "r0"] {
        // Upload/create are the only media upload endpoints on this server
        // (there is no authenticated upload under `/_matrix/client/v1/media`),
        // so they stay mounted whenever legacy media is enabled. `freeze_legacy`
        // only withholds the outbound-fetching `preview_url` endpoint.
        let mut authed_routes = Router::with_path(v)
            .hoop(hoops::auth_by_access_token)
            .push(Router::with_path("create").post(create_mxc_uri))
            .push(
                Router::with_path("upload")
                    .post(create_content)
                    .push(Router::with_path("{server_name}/{media_id}").put(upload_content)),
            )
            .push(
                Router::with_hoop(hoops::limit_rate)
                    .push(Router::with_path("config").get(get_config)),
            );

        if legacy_url_preview_enabled(media_config) {
            authed_routes = authed_routes.push(
                Router::with_hoop(hoops::limit_rate)
                    .push(Router::with_path("preview_url").get(preview_url)),
            );
        }

        media = media.push(authed_routes).push(
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

#[cfg(test)]
mod tests {
    use super::{legacy_media_enabled, legacy_url_preview_enabled};
    use crate::config::MediaConfig;

    fn media_config(allow_legacy: bool, freeze_legacy: bool) -> MediaConfig {
        MediaConfig {
            allow_legacy,
            freeze_legacy,
            ..Default::default()
        }
    }

    #[test]
    fn legacy_media_can_be_disabled_entirely() {
        let media = media_config(false, false);

        assert!(!legacy_media_enabled(&media));
        assert!(!legacy_url_preview_enabled(&media));
    }

    #[test]
    fn frozen_legacy_media_keeps_uploads_but_hides_url_preview() {
        // Default posture: legacy media on, frozen. Uploads must remain
        // available; only the URL preview endpoint is withheld.
        let media = media_config(true, true);

        assert!(legacy_media_enabled(&media));
        assert!(!legacy_url_preview_enabled(&media));
    }

    #[test]
    fn unfrozen_legacy_media_exposes_url_preview() {
        let media = media_config(true, false);

        assert!(legacy_media_enabled(&media));
        assert!(legacy_url_preview_enabled(&media));
    }
}
