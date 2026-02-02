use std::fmt::Write;
/// Endpoints for the media repository.
use std::time::Duration;

use bytes::BytesMut;
use reqwest::Url;
use salvo::oapi::{ToParameters, ToSchema};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::http_headers::ContentDisposition;
use crate::media::ResizeMethod;
use crate::sending::{SendRequest, SendResult};

/// The `multipart/mixed` mime "essence".
const MULTIPART_MIXED: &str = "multipart/mixed";
/// The maximum number of headers to parse in a body part.
const MAX_HEADERS_COUNT: usize = 32;
/// The length of the generated boundary.
const GENERATED_BOUNDARY_LENGTH: usize = 30;

/// `/v1/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1mediathumbnailmediaid
pub fn thumbnail_request(origin: &str, args: ThumbnailReqArgs) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/media/thumbnail/{}",
        args.media_id
    ))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("width", &args.width.to_string());
        query.append_pair("height", &args.height.to_string());
        query.append_pair("timeout_ms", &args.timeout_ms.as_millis().to_string());
    }
    Ok(crate::sending::get(url))
}

/// Request type for the `get_content_thumbnail` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ThumbnailReqArgs {
    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The desired resizing method.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

    /// The maximum duration that the client is willing to wait to start
    /// receiving data, in the case that the content has not yet been
    /// uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,

    /// Whether the server should return an animated thumbnail.
    ///
    /// When `Some(true)`, the server should return an animated thumbnail if
    /// possible and supported. When `Some(false)`, the server must not
    /// return an animated thumbnail. When `None`, the server should not
    /// return an animated thumbnail.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
}

/// Response type for the `get_content_thumbnail` endpoint.
#[derive(ToSchema, Debug)]
pub struct ThumbnailResBody {
    /// The metadata of the thumbnail.
    pub metadata: ContentMetadata,

    /// The content of the thumbnail.
    pub content: FileOrLocation,
}

impl Scribe for ThumbnailResBody {
    /// Serialize the given metadata and content into a `http::Response`
    /// `multipart/mixed` body.
    ///
    /// Returns a tuple containing the boundary used
    fn render(self, res: &mut Response) {
        use rand::Rng as _;

        let boundary = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .map(char::from)
            .take(GENERATED_BOUNDARY_LENGTH)
            .collect::<String>();

        let mut body_writer = BytesMut::new();

        // Add first boundary separator and header for the metadata.
        let _ = write!(
            body_writer,
            "\r\n--{boundary}\r\n{}: {}\r\n\r\n",
            http::header::CONTENT_TYPE,
            mime::APPLICATION_JSON
        );

        // Add serialized metadata.
        match serde_json::to_vec(&self.metadata) {
            Ok(bytes) => {
                body_writer.extend_from_slice(&bytes);
            }
            Err(e) => {
                error!("Failed to serialize metadata: {}", e);
                res.render(
                    StatusError::internal_server_error().brief("Failed to serialize metadata"),
                );
                return;
            }
        }

        // Add second boundary separator.
        let _ = write!(body_writer, "\r\n--{boundary}\r\n");

        // Add content.
        match self.content {
            FileOrLocation::File(content) => {
                // Add headers.
                let content_type = content
                    .content_type
                    .as_deref()
                    .unwrap_or(mime::APPLICATION_OCTET_STREAM.as_ref());
                let _ = write!(
                    body_writer,
                    "{}: {content_type}\r\n",
                    http::header::CONTENT_TYPE
                );

                if let Some(content_disposition) = &content.content_disposition {
                    let _ = write!(
                        body_writer,
                        "{}: {content_disposition}\r\n",
                        http::header::CONTENT_DISPOSITION
                    );
                }

                // Add empty line separator after headers.
                body_writer.extend_from_slice(b"\r\n");

                // Add bytes.
                body_writer.extend_from_slice(&content.file);
            }
            FileOrLocation::Location(location) => {
                // Only add location header and empty line separator.
                let _ = write!(
                    body_writer,
                    "{}: {location}\r\n\r\n",
                    http::header::LOCATION
                );
            }
        }

        // Add final boundary.
        let _ = write!(body_writer, "\r\n--{boundary}--");

        let content_type = format!("{MULTIPART_MIXED}; boundary={boundary}");

        let _ = res.add_header(http::header::CONTENT_TYPE, content_type, true);
        if let Err(e) = res.write_body(body_writer) {
            res.render(StatusError::internal_server_error().brief("Failed to set response body"));
            error!("Failed to set response body: {}", e);
        }
    }
}

// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1mediadownloadmediaid
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/media/r0/download/:server_name/:media_id",
//         1.1 => "/_matrix/media/v3/download/:server_name/:media_id",
//     }
// };

pub fn content_request(origin: &str, args: ContentReqArgs) -> SendResult<SendRequest> {
    let url = Url::parse(&format!(
        "{origin}/_matrix/federation/v1/media/download/{}?timeout_ms={}",
        args.media_id,
        args.timeout_ms.as_millis()
    ))?;
    Ok(crate::sending::get(url))
}

/// Request type for the `get_media_content` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ContentReqArgs {
    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The maximum duration that the client is willing to wait to start
    /// receiving data, in the case that the content has not yet been
    /// uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::client::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,
}

/// Response type for the `get_content` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ContentResBody {
    /// The metadata of the media.
    pub metadata: ContentMetadata,

    /// The content of the media.
    pub content: FileOrLocation,
}

/// A file from the content repository or the location where it can be found.
#[derive(ToSchema, Serialize, Debug, Clone)]
pub enum FileOrLocation {
    /// The content of the file.
    File(Content),

    /// The file is at the given URL.
    Location(String),
}

/// The content of a file from the content repository.
#[derive(ToSchema, Serialize, Debug, Clone)]
pub struct Content {
    /// The content of the file as bytes.
    pub file: Vec<u8>,

    /// The content type of the file that was previously uploaded.
    pub content_type: Option<String>,

    /// The value of the `Content-Disposition` HTTP header, possibly containing
    /// the name of the file that was previously uploaded.
    pub content_disposition: Option<ContentDisposition>,
}
/// The metadata of a file from the content repository.
#[derive(ToSchema, Serialize, Deserialize, Debug, Clone, Default)]
pub struct ContentMetadata {}

impl ContentMetadata {
    /// Creates a new empty `ContentMetadata`.
    pub fn new() -> Self {
        Self {}
    }
}
