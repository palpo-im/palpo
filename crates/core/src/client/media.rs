/// Endpoints for the media repository.
mod content;
use std::time::Duration;

pub use content::*;
use reqwest::Url;
use salvo::oapi::{ToParameters, ToSchema};
use serde::{Deserialize, Serialize};

use crate::media::ResizeMethod;
use crate::sending::{SendRequest, SendResult};
use crate::serde::{RawJsonValue, to_raw_json_value};
use crate::{OwnedMxcUri, OwnedServerName, ServerName, UnixMillis};

/// The default duration that the client should be willing to wait to start
/// receiving data.
pub(crate) fn default_download_timeout() -> Duration {
    Duration::from_secs(20)
}

/// Whether the given duration is the default duration that the client should be
/// willing to wait to start receiving data.
pub(crate) fn is_default_download_timeout(timeout: &Duration) -> bool {
    timeout.as_secs() == 20
}

// /// `POST /_matrix/media/*/create`
// ///
// /// Create an MXC URI without content.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixmediav1create

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/media/unstable/fi.mau.msc2246/create",
//         1.7 => "/_matrix/media/v1/create",
//     }
// };
/// Response type for the `create_mxc_uri` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct CreateMxcUriResBody {
    /// The MXC URI for the about to be uploaded content.
    pub content_uri: OwnedMxcUri,

    /// The time at which the URI will expire if an upload has not been started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unused_expires_at: Option<UnixMillis>,
}

// impl CreateMxcUriResBody {
//     /// Creates a new `Response` with the given MXC URI.
//     pub fn new(content_uri: OwnedMxcUri) -> Self {
//         Self {
//             content_uri,
//             unused_expires_at: None,
//         }
//     }
// }

// /// `GET /_matrix/media/*/config`
// ///
// /// Gets the config for the media repository.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixmediav3config

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/media/r0/config",
//         1.1 => "/_matrix/media/v3/config",
//     }
// };

/// Response type for the `get_media_config` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ConfigResBody {
    /// Maximum size of upload in bytes.
    #[serde(rename = "m.upload.size")]
    pub upload_size: u64,
}

impl ConfigResBody {
    /// Creates a new `Response` with the given maximum upload size.
    pub fn new(upload_size: u64) -> Self {
        Self { upload_size }
    }
}
// /// `GET /_matrix/media/*/preview_url`
// ///
// /// Get a preview for a URL.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixmediav3preview_url
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/media/r0/preview_url",
//         1.1 => "/_matrix/media/v3/preview_url",
//     }
// };

/// Request type for the `get_media_preview` endpoint.

#[derive(ToParameters, Deserialize, Debug)]
pub struct MediaPreviewReqArgs {
    /// URL to get a preview of.
    #[salvo(parameter(parameter_in = Query))]
    pub url: String,

    /// Preferred point in time (in milliseconds) to return a preview for.
    #[salvo(parameter(parameter_in = Query))]
    pub ts: Option<UnixMillis>,
}

/// Response type for the `get_media_preview` endpoint.
#[derive(ToSchema, Serialize, Default, Debug)]
#[salvo(schema(value_type = Object))]
pub struct MediaPreviewResBody(
    /// OpenGraph-like data for the URL.
    ///
    /// Differences from OpenGraph: the image size in bytes is added to the
    /// `matrix:image:size` field, and `og:image` returns the MXC URI to the
    /// image, if any.
    pub Option<Box<RawJsonValue>>,
);
impl MediaPreviewResBody {
    /// Creates an empty `Response`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Creates a new `Response` with the given OpenGraph data (in a
    /// `serde_json::value::RawValue`).
    pub fn from_raw_value(data: Box<RawJsonValue>) -> Self {
        Self(Some(data))
    }

    /// Creates a new `Response` with the given OpenGraph data (in any kind of
    /// serializable object).
    pub fn from_serialize<T: Serialize>(data: &T) -> serde_json::Result<Self> {
        Ok(Self(Some(to_raw_json_value(data)?)))
    }
}

pub fn thumbnail_request(
    origin: &str,
    server: &ServerName,
    args: ThumbnailReqArgs,
) -> SendResult<SendRequest> {
    let mut url = Url::parse(&format!(
        "{origin}/_matrix/media/v3/thumbnail/{server}/{}",
        args.media_id
    ))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("width", &args.width.to_string());
        query.append_pair("height", &args.height.to_string());
        query.append_pair("allow_remote", &args.allow_remote.to_string());
        query.append_pair("timeout_ms", &args.timeout_ms.as_millis().to_string());
        query.append_pair("allow_redirect", &args.allow_redirect.to_string());
    }
    Ok(crate::sending::get(url))
}

/// Request type for the `get_content_thumbnail` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ThumbnailReqArgs {
    /// The server name from the mxc:// URI (the authoritory component).
    #[salvo(parameter(parameter_in = Path))]
    pub server_name: OwnedServerName,

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
    #[salvo(parameter(parameter_in = Query))]
    #[serde(
        with = "crate::serde::duration::ms",
        default = "crate::client::media::default_download_timeout",
        skip_serializing_if = "crate::media::is_default_download_timeout"
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

// /// Response type for the `get_content_thumbnail` endpoint.
// #[derive(ToSchema, Serialize, Debug)]
// pub struct ThumbnailResBody {
//     /// A thumbnail of the requested content.
//     #[palpo_api(raw_body)]
//     pub file: Vec<u8>,

//     /// The content type of the thumbnail.
//     #[palpo_api(header = CONTENT_TYPE)]
//     pub content_type: Option<String>,

//     /// The value of the `Cross-Origin-Resource-Policy` HTTP header.
//     ///
//     /// See [MDN] for the syntax.
//     ///
//     /// [MDN]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cross-Origin-Resource-Policy#syntax
//     #[palpo_api(header = CROSS_ORIGIN_RESOURCE_POLICY)]
//     pub cross_origin_resource_policy: Option<String>,
// }
// impl ThumbnailResBody {
//     /// Creates a new `Response` with the given thumbnail.
//     ///
//     /// The Cross-Origin Resource Policy defaults to `cross-origin`.
//     pub fn new(file: Vec<u8>) -> Self {
//             file,
//             content_type: None,
//             cross_origin_resource_policy: Some("cross-origin".to_owned()),
//         }
//         Self {
//     }
// }

// #[cfg(test)]
// mod tests {
//     use crate::RawJsonValue;
//     use assert_matches2::assert_matches;
//     use serde_json::{from_value as from_json_value, json, value::to_raw_value
// as to_raw_json_value};

//     // Since BTreeMap<String, Box<RawJsonValue>> deserialization doesn't seem
// to     // work, test that Option<RawJsonValue> works
//     #[test]
//     fn raw_json_deserialize() {
//         type OptRawJson = Option<Box<RawJsonValue>>;

//         assert_matches!(from_json_value::<OptRawJson>(json!(null)).unwrap(),
// None);         from_json_value::<OptRawJson>(json!("test")).unwrap().
// unwrap();         from_json_value::<OptRawJson>(json!({ "a": "b"
// })).unwrap().unwrap();     }

//     // For completeness sake, make sure serialization works too
//     #[test]
//     fn raw_json_serialize() {
//         to_raw_json_value(&json!(null)).unwrap();
//         to_raw_json_value(&json!("string")).unwrap();
//         to_raw_json_value(&json!({})).unwrap();
//         to_raw_json_value(&json!({ "a": "b" })).unwrap();
//     }
// }
