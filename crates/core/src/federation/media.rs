use std::fmt::Write;
use std::str::FromStr;
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
        use rand::RngExt as _;

        let mut rng = rand::rng();
        let boundary = (&mut rng)
            .sample_iter(rand::distr::Alphanumeric)
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

/// An error encountered while parsing the `multipart/mixed` body of a
/// federation media response.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MultipartMixedError {
    /// The `Content-Type` header is not a valid mime type.
    #[error("the `Content-Type` header is not a valid mime type: `{0}`")]
    InvalidContentType(String),

    /// The `Content-Type` header is not `multipart/mixed`.
    #[error("expected a `multipart/mixed` response, got `{0}`")]
    NotMultipartMixed(String),

    /// The `Content-Type` header is missing the `boundary` parameter.
    #[error("the `Content-Type` header is missing the `boundary` parameter")]
    MissingBoundary,

    /// The body does not contain the two expected body parts.
    #[error("expected 2 body parts, found {found}")]
    MissingBodyParts {
        /// The number of body parts that could be found.
        found: usize,
    },

    /// A body part is missing the empty line that separates its headers from
    /// its content.
    #[error("a body part is missing the separator between its headers and its content")]
    MissingBodyPartSeparator,

    /// A body part has headers that are not valid UTF-8.
    #[error("a body part has headers that are not valid UTF-8")]
    InvalidHeaders,
}

/// Deserialize the metadata and content of a federation media response with a
/// `multipart/mixed` body, as described in the [spec].
///
/// `content_type` is the value of the response's `Content-Type` header, and
/// `body` its full body.
///
/// [spec]: https://spec.matrix.org/latest/server-server-api/#get_matrixfederationv1mediadownloadmediaid
pub fn try_from_multipart_mixed(
    content_type: &str,
    body: &[u8],
) -> Result<(ContentMetadata, FileOrLocation), MultipartMixedError> {
    let mime = content_type
        .parse::<mime::Mime>()
        .map_err(|_| MultipartMixedError::InvalidContentType(content_type.to_owned()))?;

    if !mime.essence_str().eq_ignore_ascii_case(MULTIPART_MIXED) {
        return Err(MultipartMixedError::NotMultipartMixed(
            mime.essence_str().to_owned(),
        ));
    }

    let boundary = mime
        .get_param(mime::BOUNDARY)
        .ok_or(MultipartMixedError::MissingBoundary)?;

    // The delimiter between body parts is the boundary preceded by a CRLF. The
    // closing delimiter starts with the same bytes, so searching for this also
    // finds the end of the last part.
    let mut delimiter = Vec::with_capacity(boundary.as_str().len() + 4);
    delimiter.extend_from_slice(b"\r\n--");
    delimiter.extend_from_slice(boundary.as_str().as_bytes());
    let delimiter_no_crlf = &delimiter[2..];

    // If there is no preamble before the first delimiter, it may omit the
    // preceding CRLF.
    let metadata_start = if body.starts_with(delimiter_no_crlf) {
        delimiter_no_crlf.len()
    } else {
        find_from(body, &delimiter, 0).ok_or(MultipartMixedError::MissingBodyParts { found: 0 })?
            + delimiter.len()
    };
    let metadata_end = find_from(body, &delimiter, metadata_start)
        .ok_or(MultipartMixedError::MissingBodyParts { found: 0 })?;

    // Don't look at the headers of the metadata part, its content is always
    // JSON. It carries no field yet, so a body we cannot parse is not worth
    // failing the whole response over.
    let (_, raw_metadata) = parse_body_part(body, metadata_start, metadata_end)?;
    let metadata = serde_json::from_slice(raw_metadata).unwrap_or_default();

    // The second part holds the file itself, or the location where it can be
    // found.
    let content_start = metadata_end + delimiter.len();
    let content_end = find_from(body, &delimiter, content_start)
        .ok_or(MultipartMixedError::MissingBodyParts { found: 1 })?;

    let (raw_headers, file) = parse_body_part(body, content_start, content_end)?;
    let headers = parse_body_part_headers(raw_headers)?;

    let content = if let Some(location) = headers.location {
        FileOrLocation::Location(location)
    } else {
        FileOrLocation::File(Content {
            file: file.to_owned(),
            content_type: headers.content_type,
            content_disposition: headers.content_disposition,
        })
    };

    Ok((metadata, content))
}

/// Find the first occurrence of `needle` in `haystack`, starting the search at
/// `start`. The returned position is relative to the start of `haystack`.
fn find_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if start > haystack.len() {
        return None;
    }

    haystack[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|pos| pos + start)
}

/// Parse the body part in the given bytes, starting and ending at the given
/// positions.
///
/// Returns a `(headers_bytes, content_bytes)` tuple.
fn parse_body_part(
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Result<(&[u8], &[u8]), MultipartMixedError> {
    // The part starts with the rest of the delimiter line. We need to ignore
    // characters before its line feed in case of extra whitespace, and for
    // compatibility it might not have a carriage return.
    let headers_start = find_line_feed(bytes, start, end)? + 1;

    // Then the headers end at the first empty line.
    let mut line_start = headers_start;
    loop {
        let line_end = find_line_feed(bytes, line_start, end)? + 1;

        if matches!(&bytes[line_start..line_end], b"\r\n" | b"\n") {
            return Ok((&bytes[headers_start..line_start], &bytes[line_end..end]));
        }

        line_start = line_end;
    }
}

/// Find the position of the next line feed in `bytes[start..end]`.
fn find_line_feed(bytes: &[u8], start: usize, end: usize) -> Result<usize, MultipartMixedError> {
    bytes
        .get(start..end)
        .and_then(|bytes| bytes.iter().position(|byte| *byte == b'\n'))
        .map(|pos| pos + start)
        .ok_or(MultipartMixedError::MissingBodyPartSeparator)
}

/// The headers of a body part that palpo cares about.
#[derive(Default)]
struct BodyPartHeaders {
    location: Option<String>,
    content_type: Option<String>,
    content_disposition: Option<ContentDisposition>,
}

fn parse_body_part_headers(raw: &[u8]) -> Result<BodyPartHeaders, MultipartMixedError> {
    let raw = std::str::from_utf8(raw).map_err(|_| MultipartMixedError::InvalidHeaders)?;
    let mut headers = BodyPartHeaders::default();

    for line in raw.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());

        if name.eq_ignore_ascii_case(http::header::LOCATION.as_str()) {
            headers.location = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case(http::header::CONTENT_TYPE.as_str()) {
            headers.content_type = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case(http::header::CONTENT_DISPOSITION.as_str()) {
            // A `Content-Disposition` we cannot parse only costs us the
            // filename, so it is not worth rejecting the response.
            headers.content_disposition = ContentDisposition::from_str(value).ok();
        }
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use salvo::http::ResBody;
    use salvo::prelude::Scribe;

    use super::{
        Content, ContentMetadata, FileOrLocation, MultipartMixedError, ThumbnailResBody,
        try_from_multipart_mixed,
    };
    use crate::http_headers::{ContentDisposition, ContentDispositionType};

    const BOUNDARY: &str = "multipart/mixed; boundary=abcdef";

    fn file(content: FileOrLocation) -> Content {
        match content {
            FileOrLocation::File(content) => content,
            FileOrLocation::Location(location) => {
                panic!("expected a file, got the location `{location}`")
            }
        }
    }

    #[test]
    fn parse_simple_response() {
        let body = "\r\n--abcdef\r\ncontent-type: application/json\r\n\r\n{}\r\n--abcdef\r\n\
                    content-type: text/plain\r\n\r\nsome plain text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type.unwrap(), "text/plain");
        assert_eq!(content.content_disposition, None);
    }

    #[test]
    fn parse_headers_case_insensitively() {
        let body = "\r\n--abcdef\r\nCONTENT-type: application/json\r\n\r\n{}\r\n--abcdef\r\n\
                    CONTENT-TYPE: text/plain\r\ncoNtenT-disPosItioN: attachment; \
                    filename=my_file.txt\r\n\r\nsome plain text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type.unwrap(), "text/plain");
        let content_disposition = content.content_disposition.unwrap();
        assert_eq!(
            content_disposition.disposition_type,
            ContentDispositionType::Attachment
        );
        assert_eq!(content_disposition.filename.unwrap(), "my_file.txt");
    }

    #[test]
    fn parse_quoted_and_uppercase_boundary_param() {
        let body = "\r\n--abcdef\r\ncontent-type: application/json\r\n\r\n{}\r\n--abcdef\r\n\
                    content-type: text/plain\r\n\r\nsome plain text\r\n--abcdef--";

        let (_metadata, content) =
            try_from_multipart_mixed("multipart/mixed; BOUNDARY=\"abcdef\"", body.as_bytes())
                .unwrap();

        assert_eq!(file(content).file, b"some plain text");
    }

    #[test]
    fn parse_response_with_extra_whitespace() {
        let body = "   \r\n--abcdef\r\ncontent-type:   application/json   \r\n\r\n {} \
                    \r\n--abcdef\r\ncontent-type: text/plain  \r\n\r\nsome plain \
                    text\r\n--abcdef--  ";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type.unwrap(), "text/plain");
    }

    #[test]
    fn parse_response_without_carriage_returns_inside_parts() {
        let body = "\r\n--abcdef\ncontent-type: application/json\n\n{}\r\n--abcdef\n\
                    content-type: text/plain  \n\nsome plain text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type.unwrap(), "text/plain");
    }

    #[test]
    fn parse_response_without_leading_crlf() {
        let body = "--abcdef\r\n\r\n{}\r\n--abcdef\r\n\r\nsome plain text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type, None);
    }

    #[test]
    fn parse_response_with_boundary_text_in_preamble() {
        // The preamble has no leading CRLF before the boundary, so it must be
        // ignored.
        let body = "foo--abcdef\r\n--abcdef\r\n\r\n{}\r\n--abcdef\r\n\r\nsome plain \
                    text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();

        assert_eq!(file(content).file, b"some plain text");
    }

    #[test]
    fn parse_body_parts_without_headers() {
        let body = "\r\n--abcdef\r\n\r\n{}\r\n--abcdef\r\n\r\nsome plain text\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"some plain text");
        assert_eq!(content.content_type, None);
        assert_eq!(content.content_disposition, None);
    }

    #[test]
    fn parse_binary_content_containing_crlf() {
        // The file must be returned byte for byte, even when it contains the
        // sequences the parser looks for.
        let file_bytes = b"\r\n--abcde\r\n\r\nnot a boundary\r\n";
        let mut body = b"\r\n--abcdef\r\ncontent-type: application/json\r\n\r\n{}\r\n--abcdef\r\n\
                         content-type: application/octet-stream\r\n\r\n"
            .to_vec();
        body.extend_from_slice(file_bytes);
        body.extend_from_slice(b"\r\n--abcdef--");

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, &body).unwrap();

        assert_eq!(file(content).file, file_bytes);
    }

    #[test]
    fn parse_location_response() {
        let body = "\r\n--abcdef\r\ncontent-type: application/json\r\n\r\n{}\r\n--abcdef\r\n\
                    location: https://server.local/media/filename.txt\r\n\r\n\r\n--abcdef--";

        let (_metadata, content) = try_from_multipart_mixed(BOUNDARY, body.as_bytes()).unwrap();

        let FileOrLocation::Location(location) = content else {
            panic!("expected a location");
        };
        assert_eq!(location, "https://server.local/media/filename.txt");
    }

    #[test]
    fn reject_invalid_responses() {
        let body = "\r\n--abcdef\r\n\r\n{}\r\n--abcdef\r\nContent-Type: text/plain\r\n\r\nsome \
                    plain text\r\n--abcdef--";

        // Not a multipart response.
        assert!(matches!(
            try_from_multipart_mixed("text/plain", body.as_bytes()),
            Err(MultipartMixedError::NotMultipartMixed(_))
        ));

        // Missing boundary parameter.
        assert!(matches!(
            try_from_multipart_mixed("multipart/mixed", body.as_bytes()),
            Err(MultipartMixedError::MissingBoundary)
        ));

        // Wrong boundary.
        assert!(matches!(
            try_from_multipart_mixed("multipart/mixed; boundary=012345", body.as_bytes()),
            Err(MultipartMixedError::MissingBodyParts { found: 0 })
        ));

        // Missing the closing delimiter.
        let body = "\r\n--abcdef\r\n\r\n{}\r\n--abcdef\r\nContent-Type: text/plain\r\n\r\nsome \
                    plain text";
        assert!(matches!(
            try_from_multipart_mixed(BOUNDARY, body.as_bytes()),
            Err(MultipartMixedError::MissingBodyParts { found: 1 })
        ));

        // Missing the empty line between the headers and the content of a part.
        let body = "\r\n--abcdef\r\n{}\r\n--abcdef\r\nContent-Type: text/plain\r\nsome plain \
                    text\r\n--abcdef--";
        assert!(matches!(
            try_from_multipart_mixed(BOUNDARY, body.as_bytes()),
            Err(MultipartMixedError::MissingBodyPartSeparator)
        ));
    }

    #[test]
    fn round_trip_rendered_response() {
        // What palpo renders must be what palpo parses.
        let content_disposition = ContentDisposition::new(ContentDispositionType::Inline)
            .with_filename(Some("fȈlƩnąmǝ.txt".to_owned()));
        let mut res = salvo::Response::new();
        ThumbnailResBody {
            metadata: ContentMetadata::new(),
            content: FileOrLocation::File(Content {
                file: b"s\xffme UTF-8 \xc5\xa4ext".to_vec(),
                content_type: Some("text/plain".to_owned()),
                content_disposition: Some(content_disposition.clone()),
            }),
        }
        .render(&mut res);

        let rendered_content_type = res
            .headers()
            .get(http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let ResBody::Once(rendered_body) = &res.body else {
            panic!("expected a rendered body");
        };

        let (_metadata, content) =
            try_from_multipart_mixed(&rendered_content_type, rendered_body).unwrap();
        let content = file(content);

        assert_eq!(content.file, b"s\xffme UTF-8 \xc5\xa4ext");
        assert_eq!(content.content_type.unwrap(), "text/plain");
        assert_eq!(content.content_disposition.unwrap(), content_disposition);
    }
}
