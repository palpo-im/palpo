//! `/unstable/org.matrix.msc3916.v2/` ([MSC])
//!
//! [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/3916

use std::time::Duration;

use salvo::oapi::ToParameters;
use serde::Deserialize;

/// Request type for the `get_content` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct GetMediaContentArgs {
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

impl GetMediaContentArgs {
    /// Creates a new `GetMediaContentArgs` with the given media ID.
    pub fn new(media_id: String) -> Self {
        Self {
            media_id,
            timeout_ms: crate::media::default_download_timeout(),
        }
    }
}

// impl Metadata for Request {
//     const METHOD: http::Method = super::v1::Request::METHOD;
//     const RATE_LIMITED: bool = super::v1::Request::RATE_LIMITED;
//     type Authentication = <super::v1::Request as Metadata>::Authentication;
//     type PathBuilder = <super::v1::Request as Metadata>::PathBuilder;
//     const PATH_BUILDER: Self::PathBuilder = SinglePath::new(
//         "/_matrix/federation/unstable/org.matrix.msc3916.v2/media/download/{media_id}",
//     );
// }

// impl From<super::v1::Request> for Request {
//     fn from(value: super::v1::Request) -> Self {
//         let super::v1::Request {
//             media_id,
//             timeout_ms,
//         } = value;
//         Self {
//             media_id,
//             timeout_ms,
//         }
//     }
// }

// impl From<Request> for super::v1::Request {
//     fn from(value: Request) -> Self {
//         let Request {
//             media_id,
//             timeout_ms,
//         } = value;
//         Self {
//             media_id,
//             timeout_ms,
//         }
//     }
// }

// /// Response type for the `get_content` endpoint.
// #[derive(ToSchema, Serialize, Clone, Debug)]
// pub struct GetMediaContentResBody {
//     /// The metadata of the media.
//     pub metadata: ContentMetadata,

//     /// The content of the media.
//     pub content: FileOrLocation,
// }

// impl GetMediaContentResBody {
//     /// Creates a new `GetMediaContentResBody` with the given metadata and content.
//     pub fn new(metadata: ContentMetadata, content: FileOrLocation) -> Self {
//         Self { metadata, content }
//     }
// }

// #[cfg(feature = "client")]
// impl crate::api::IncomingResponse for Response {
//     type EndpointError = <super::v1::Response as crate::api::IncomingResponse>::EndpointError;

//     fn try_from_http_response<T: AsRef<[u8]>>(
//         http_response: http::Response<T>,
//     ) -> Result<Self, crate::api::error::FromHttpResponseError<Self::EndpointError>> {
//         // Reuse the custom deserialization.
//         Ok(super::v1::Response::try_from_http_response(http_response)?.into())
//     }
// }

// #[cfg(feature = "server")]
// impl crate::api::OutgoingResponse for Response {
//     fn try_into_http_response<T: Default + bytes::BufMut>(
//         self,
//     ) -> Result<http::Response<T>, crate::api::error::IntoHttpError> {
//         // Reuse the custom serialization.
//         super::v1::Response::from(self).try_into_http_response()
//     }
// }

// impl From<super::v1::Response> for Response {
//     fn from(value: super::v1::Response) -> Self {
//         let super::v1::Response { metadata, content } = value;
//         Self { metadata, content }
//     }
// }

// impl From<Response> for super::v1::Response {
//     fn from(value: Response) -> Self {
//         let Response { metadata, content } = value;
//         Self { metadata, content }
//     }
// }
