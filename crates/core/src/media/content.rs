/// `GET /_matrix/media/*/download/{serverName}/{mediaId}`
///
/// Retrieve content from the media store.
use std::time::Duration;

use reqwest::Url;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

// use crate::http_headers::CROSS_ORIGIN_RESOURCE_POLICY;
use crate::sending::{SendRequest, SendResult};
use crate::{OwnedMxcUri, OwnedServerName};

// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixmediav3downloadservernamemediaid
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
        "{origin}/_matrix/media/v3/download/{}/{}?allow_remote={}&allow_redirect={}",
        args.server_name, args.media_id, args.allow_remote, args.allow_redirect
    ))?;
    Ok(crate::sending::get(url))
}

/// Request type for the `get_media_content` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ContentReqArgs {
    /// The server name from the mxc:// URI (the authoritory component).
    #[salvo(parameter(parameter_in = Path))]
    pub server_name: OwnedServerName,

    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// Whether to fetch media deemed remote.
    ///
    /// Used to prevent routing loops. Defaults to `true`.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub allow_remote: bool,

    /// The maximum duration that the client is willing to wait to start
    /// receiving data, in the case that the content has not yet been
    /// uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query, value_type = u64))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::client::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,

    /// Whether the server may return a 307 or 308 redirect response that points
    /// at the relevant media content.
    ///
    /// Unless explicitly set to `true`, the server must return the media
    /// content itself.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allow_redirect: bool,
}

#[derive(ToParameters, Deserialize, Debug)]
pub struct ContentWithFileNameReqArgs {
    /// The server name from the mxc:// URI (the authoritory component).
    #[salvo(parameter(parameter_in = Path))]
    pub server_name: OwnedServerName,

    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    #[salvo(parameter(parameter_in = Path))]
    pub filename: String,

    /// Whether to fetch media deemed remote.
    ///
    /// Used to prevent routing loops. Defaults to `true`.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub allow_remote: bool,

    /// The maximum duration that the client is willing to wait to start
    /// receiving data, in the case that the content has not yet been
    /// uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query, value_type = u64))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::client::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,

    /// Whether the server may return a 307 or 308 redirect response that points
    /// at the relevant media content.
    ///
    /// Unless explicitly set to `true`, the server must return the media
    /// content itself.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allow_redirect: bool,
}

// /// Response type for the `get_media_content` endpoint.
// #[derive(ToSchema, Serialize, Debug)]
// pub struct Content {
//     /// The content that was previously uploaded.
//     pub data: Vec<u8>,

//     /// The content type of the file that was previously uploaded.
//     pub content_type: Option<String>,

//     /// The value of the `Content-Disposition` HTTP header, possibly
// containing the name of the     /// file that was previously uploaded.
//     ///
//     /// See [MDN] for the syntax.
//     ///
//     /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Disposition#Syntax
//     pub content_disposition: Option<String>,

//     /// The value of the `Cross-Origin-Resource-Policy` HTTP header.
//     ///
//     /// See [MDN] for the syntax.
//     ///
//     /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cross-Origin-Resource-Policy#syntax
//     pub cross_origin_resource_policy: Option<String>,
// }
// impl Content {
//     /// Creates a new `Response` with the given file contents.
//     ///
//     /// The Cross-Origin Resource Policy defaults to `cross-origin`.
//     pub fn new(data: Vec<u8>) -> Self {
//         Self {
//             data,
//             content_type: None,
//             content_disposition: None,
//             cross_origin_resource_policy: Some("cross-origin".to_owned()),
//         }
//     }
// }

// impl Scribe for Content {
//     fn render(self, res: &mut Response) {
//         let Self {
//             data,
//             content_type,
//             content_disposition,
//             cross_origin_resource_policy,
//         } = self;
//         if let Some(content_type) = content_type {
//             res.add_header(CONTENT_TYPE, content_type, true);
//         }
//         if let Some(content_disposition) = content_disposition {
//             res.add_header(CONTENT_DISPOSITION, content_disposition, true);
//         }
//         if let Some(cross_origin_resource_policy) =
// cross_origin_resource_policy {
// res.add_header("cross-origin-resource-policy", cross_origin_resource_policy,
// true);         }
//         res.write_body(data);
//     }
// }

/// `POST /_matrix/media/*/upload`
///
/// Upload content to the media store.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixmediav3upload

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/media/r0/upload",
//         1.1 => "/_matrix/media/v3/upload",
//     }
// };

#[derive(ToParameters, Deserialize, Debug)]
pub struct CreateContentReqArgs {
    /// The name of the file being uploaded.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// The content type of the file being uploaded.
    #[serde(rename = "content-type")]
    #[salvo(parameter(parameter_in = Header))]
    pub content_type: Option<String>,

    /// Should the server return a blurhash or not.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        default,
        skip_serializing_if = "crate::serde::is_default",
        rename = "xyz.amorgan.generate_blurhash"
    )]
    pub generate_blurhash: bool,
}

/// Response type for the `create_media_content` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct CreateContentResBody {
    /// The MXC URI for the uploaded content.
    pub content_uri: OwnedMxcUri,

    /// The [BlurHash](https://blurha.sh) for the uploaded content.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    pub blurhash: Option<String>,
}
impl CreateContentResBody {
    /// Creates a new `Response` with the given MXC URI.
    pub fn new(content_uri: OwnedMxcUri) -> Self {
        Self {
            content_uri,
            blurhash: None,
        }
    }
}
// /// `PUT /_matrix/media/*/upload/{serverName}/{mediaId}`
// ///
// /// Upload media to an MXC URI that was created with create_mxc_uri.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixmediav3uploadservernamemediaid

// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/media/unstable/fi.mau.msc2246/upload/:server_name/:media_id",
//         1.7 => "/_matrix/media/v3/upload/:server_name/:media_id",
//     }
// };

/// Request type for the `create_content_async` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct UploadContentReqArgs {
    /// The server name from the mxc:// URI (the authoritory component).
    #[salvo(parameter(parameter_in = Path))]
    pub server_name: OwnedServerName,

    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The file contents to upload.
    // #[palpo_api(raw_body)]
    // pub file: Vec<u8>,

    /// The content type of the file being uploaded.
    #[salvo(rename="content-type", parameter(parameter_in = Header))]
    pub content_type: Option<String>,

    /// The name of the file being uploaded.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    // TODO: How does this and msc2448 (blurhash) interact?
}
// /// `GET /_matrix/media/*/download/{serverName}/{mediaId}/{fileName}`
// ///
// /// Retrieve content from the media store, specifying a filename to return.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixmediav3downloadservernamemediaidfilename
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/media/r0/download/:server_name/:media_id/:filename",
//         1.1 => "/_matrix/media/v3/download/:server_name/:media_id/:filename",
//     }
// };

/// Request type for the `get_media_content_as_filename` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ContentAsFileNameReqArgs {
    /// The server name from the mxc:// URI (the authoritory component).
    #[salvo(parameter(parameter_in = Path))]
    pub server_name: OwnedServerName,

    /// The media ID from the mxc:// URI (the path component).
    #[salvo(parameter(parameter_in = Path))]
    pub media_id: String,

    /// The filename to return in the `Content-Disposition` header.
    #[salvo(parameter(parameter_in = Path))]
    pub filename: String,

    /// Whether to fetch media deemed remote.
    ///
    /// Used to prevent routing loops. Defaults to `true`.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub allow_remote: bool,

    /// The maximum duration that the client is willing to wait to start
    /// receiving data, in the case that the content has not yet been
    /// uploaded.
    ///
    /// The default value is 20 seconds.
    #[salvo(parameter(parameter_in = Query, value_type = u64))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::client::media::is_default_download_timeout"
    )]
    pub timeout_ms: Duration,

    /// Whether the server may return a 307 or 308 redirect response that points
    /// at the relevant media content.
    ///
    /// Unless explicitly set to `true`, the server must return the media
    /// content itself.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub allow_redirect: bool,
}

// /// Response type for the `get_media_content_as_filename` endpoint.
// #[derive(ToSchema, Serialize, Debug)]
// pub struct ContentAsFileNameResBody {
//     /// The content that was previously uploaded.
//     #[palpo_api(raw_body)]
//     pub file: Vec<u8>,

//     /// The content type of the file that was previously uploaded.
//     #[palpo_api(header = CONTENT_TYPE)]
//     pub content_type: Option<String>,

//     /// The value of the `Content-Disposition` HTTP header, possibly
// containing the name of the     /// file that was previously uploaded.
//     ///
//     /// See [MDN] for the syntax.
//     ///
//     /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Disposition#Syntax
//     #[palpo_api(header = CONTENT_DISPOSITION)]
//     pub content_disposition: Option<String>,

//     /// The value of the `Cross-Origin-Resource-Policy` HTTP header.
//     ///
//     /// See [MDN] for the syntax.
//     ///
//     /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cross-Origin-Resource-Policy#syntax
//     #[palpo_api(header = CROSS_ORIGIN_RESOURCE_POLICY)]
//     pub cross_origin_resource_policy: Option<String>,
// }
// impl ContentAsFileNameResBody {
//     /// Creates a new `Response` with the given file.
//     ///
//     /// The Cross-Origin Resource Policy defaults to `cross-origin`.
//     pub fn new(file: Vec<u8>) -> Self {
//         Self {
//             file,
//             content_type: None,
//             content_disposition: None,
//             cross_origin_resource_policy: Some("cross-origin".to_owned()),
//         }
//     }
// }

// `GET /_matrix/media/*/thumbnail/{serverName}/{mediaId}`
//
// Get a thumbnail of content from the media store.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixmediav3thumbnailservernamemediaid
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/media/r0/thumbnail/:server_name/:media_id",
//         1.1 => "/_matrix/media/v3/thumbnail/:server_name/:media_id",
//     }
// };
