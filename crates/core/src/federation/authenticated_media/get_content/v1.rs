//! `/v1/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1mediadownloadmediaid

use std::time::Duration;

use salvo::oapi::ToParameters;
use serde::Deserialize;

// metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: ServerSignatures,
//     path: "/_matrix/federation/v1/media/download/{media_id}",
// }

/// Request type for the `get_content` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct MediaDownloadArgs {
    /// The media ID from the `mxc://` URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The maximum duration that the client is willing to wait to start receiving data, in the
    /// case that the content has not yet been uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query, value_type = u64))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::media::default_download_timeout",
        skip_serializing_if = "crate::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,
}

impl MediaDownloadArgs {
    /// Creates a new `Request` with the given media ID.
    pub fn new(media_id: String) -> Self {
        Self {
            media_id,
            timeout_ms: crate::media::default_download_timeout(),
        }
    }
}

// /// Response type for the `get_content` endpoint.
// #[derive(ToSchema, Serialize, Clone, Debug)]
// pub struct MediaDownloadResBody {
//     /// The metadata of the media.
//     pub metadata: ContentMetadata,

//     /// The content of the media.
//     pub content: FileOrLocation,
// }

// impl MediaDownloadResBody {
//     /// Creates a new `Response` with the given metadata and content.
//     pub fn new(metadata: ContentMetadata, content: FileOrLocation) -> Self {
//         Self { metadata, content }
//     }
// }
