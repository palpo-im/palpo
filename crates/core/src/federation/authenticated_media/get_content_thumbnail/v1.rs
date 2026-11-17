//! `/v1/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1mediathumbnailmediaid

use std::time::Duration;

use salvo::oapi::ToParameters;
use serde::Deserialize;

use crate::media::ResizeMethod;

// metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: ServerSignatures,
//     path: "/_matrix/federation/v1/media/thumbnail/{media_id}",
// }

/// Request type for the `get_content_thumbnail` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct GetContentThumbnailArgs {
    /// The media ID from the `mxc://` URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The desired resizing method.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<ResizeMethod>,

    /// The *desired* width of the thumbnail.
    ///
    /// The actual thumbnail may not match the size specified.
    #[salvo(parameter(parameter_in = Query))]
    pub width: u32,

    /// The *desired* height of the thumbnail.
    ///
    /// The actual thumbnail may not match the size specified.
    #[salvo(parameter(parameter_in = Query))]
    pub height: u32,

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

    /// Whether the server should return an animated thumbnail.
    ///
    /// When `Some(true)`, the server should return an animated thumbnail if possible and
    /// supported. When `Some(false)`, the server must not return an animated
    /// thumbnail. When `None`, the server should not return an animated thumbnail.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
}

impl GetContentThumbnailArgs {
    /// Creates a new `GetContentThumbnailArgs` with the given media ID, desired thumbnail width
    /// and desired thumbnail height.
    pub fn new(media_id: String, width: u32, height: u32) -> Self {
        Self {
            media_id,
            method: None,
            width,
            height,
            timeout_ms: crate::media::default_download_timeout(),
            animated: None,
        }
    }
}

// /// Response type for the `get_content_thumbnail` endpoint.
// #[derive(ToSchema, Serialize, Clone, Debug)]
// pub struct GetContentThumbnailResBody {
//     /// The metadata of the thumbnail.
//     pub metadata: ContentMetadata,

//     /// The content of the thumbnail.
//     pub content: FileOrLocation,
// }

// impl GetContentThumbnailResBody {
//     /// Creates a new `GetContentThumbnailResBody` with the given metadata and content.
//     pub fn new(metadata: ContentMetadata, content: FileOrLocation) -> Self {
//         Self { metadata, content }
//     }
// }
