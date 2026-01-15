//! Errors that can be sent from the homeserver.

use std::error::Error as StdError;
use std::fmt;
use std::iter::FromIterator;
use std::num::ParseIntError;

use salvo::http::{Response, StatusCode, header};
use salvo::writing::Scribe;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

mod auth;
pub use auth::*;
mod kind;
/// Deserialize and Serialize implementations for ErrorKind.
/// Separate module because it's a lot of code.
mod kind_serde;
pub use kind::*;
use kind_serde::{ErrorCode, RetryAfter};

use crate::{MatrixVersion, RoomVersionId};

macro_rules! simple_kind_fns {
    ($($fname:ident, $kind:ident;)+) => {
        $(
            /// Create a new `MatrixError`.
            pub fn $fname(body: impl Into<ErrorBody>) -> Self {
                Self::new(ErrorKind::$kind, body)
            }
        )+
    }
}
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ErrorBody(JsonMap<String, JsonValue>);

impl From<String> for ErrorBody {
    fn from(message: String) -> Self {
        Self(JsonMap::from_iter(vec![(
            "error".to_owned(),
            json!(message),
        )]))
    }
}
impl From<&str> for ErrorBody {
    fn from(message: &str) -> Self {
        Self(JsonMap::from_iter(vec![(
            "error".to_owned(),
            json!(message),
        )]))
    }
}
impl From<JsonMap<String, JsonValue>> for ErrorBody {
    fn from(inner: JsonMap<String, JsonValue>) -> Self {
        Self(inner)
    }
}

/// A Matrix Error
#[derive(Debug, Clone)]
#[allow(clippy::exhaustive_structs)]
pub struct MatrixError {
    /// The http status code.
    pub status_code: Option<http::StatusCode>,

    /// The `WWW-Authenticate` header error message.
    pub authenticate: Option<AuthenticateError>,
    pub kind: ErrorKind,

    /// The http response's body.
    pub body: ErrorBody,
}
impl MatrixError {
    pub fn new(kind: ErrorKind, body: impl Into<ErrorBody>) -> Self {
        Self {
            status_code: None,
            authenticate: None,
            kind,
            body: body.into(),
        }
    }
    simple_kind_fns! {
        bad_alias, BadAlias;
        bad_json, BadJson;
        bad_state, BadState;
        cannot_leave_server_notice_room, CannotLeaveServerNoticeRoom;
        cannot_overwrite_media, CannotOverwriteMedia;
        captcha_invalid, CaptchaInvalid;
        captcha_needed, CaptchaNeeded;
        connection_failed, ConnectionFailed;
        connection_timeout, ConnectionTimeout;
        duplicate_annotation, DuplicateAnnotation;
        exclusive, Exclusive;
        guest_access_forbidden, GuestAccessForbidden;
        invalid_param, InvalidParam;
        invalid_room_state, InvalidRoomState;
        invalid_username, InvalidUsername;
        missing_param, MissingParam;
        missing_token, MissingToken;
        not_found, NotFound;
        not_json, NotJson;
        not_yet_uploaded, NotYetUploaded;
        room_in_use, RoomInUse;
        server_not_trusted, ServerNotTrusted;
        threepid_auth_failed, ThreepidAuthFailed;
        threepid_denied, ThreepidDenied;
        threepid_in_use, ThreepidInUse;
        threepid_medium_not_supported, ThreepidMediumNotSupported;
        threepid_not_found, ThreepidNotFound;
        too_large, TooLarge;
        unable_to_authorize_join, UnableToAuthorizeJoin;
        unable_to_grant_join, UnableToGrantJoin;
        unauthorized, Unauthorized;
        unknown, Unknown;
        unrecognized, Unrecognized;
        unsupported_room_version, UnsupportedRoomVersion;
        url_not_set, UrlNotSet;
        user_deactivated, UserDeactivated;
        user_in_use, UserInUse;
        user_locked, UserLocked;
        user_suspended, UserSuspended;
        weak_password, WeakPassword;
    }

    pub fn bad_status(status: Option<http::StatusCode>, body: impl Into<ErrorBody>) -> Self {
        Self::new(ErrorKind::BadStatus { status, body: None }, body)
    }
    pub fn forbidden(body: impl Into<ErrorBody>, authenticate: Option<AuthenticateError>) -> Self {
        Self::new(ErrorKind::Forbidden { authenticate }, body)
    }
    #[cfg(feature = "unstable-msc4186")]
    pub fn unknown_pos(body: impl Into<ErrorBody>) -> Self {
        Self::new(ErrorKind::UnknownPos, body)
    }
    #[cfg(feature = "unstable-msc3843")]
    pub fn unactionable(body: impl Into<ErrorBody>) -> Self {
        Self::new(ErrorKind::Unactionable, body)
    }
    pub fn unknown_token(body: impl Into<ErrorBody>, soft_logout: bool) -> Self {
        Self::new(ErrorKind::UnknownToken { soft_logout }, body)
    }
    pub fn limit_exceeded(body: impl Into<ErrorBody>, retry_after: Option<RetryAfter>) -> Self {
        Self::new(ErrorKind::LimitExceeded { retry_after }, body)
    }
    pub fn incompatible_room_version(
        body: impl Into<ErrorBody>,
        room_version: RoomVersionId,
    ) -> Self {
        Self::new(ErrorKind::IncompatibleRoomVersion { room_version }, body)
    }
    pub fn resource_limit_exceeded(body: impl Into<ErrorBody>, admin_contact: String) -> Self {
        Self::new(ErrorKind::ResourceLimitExceeded { admin_contact }, body)
    }
    pub fn wrong_room_keys_version(
        body: impl Into<ErrorBody>,
        current_version: Option<String>,
    ) -> Self {
        Self::new(ErrorKind::WrongRoomKeysVersion { current_version }, body)
    }
    pub fn is_not_found(&self) -> bool {
        matches!(self.kind, ErrorKind::NotFound)
    }
}
impl Serialize for MatrixError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.body.serialize(serializer)
    }
}

impl fmt::Display for MatrixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.status_code.unwrap_or(StatusCode::BAD_REQUEST).as_u16();
        write!(f, "[{code} / {}]: {:?}", self.kind.code(), self.body)
    }
}

impl StdError for MatrixError {}

impl From<serde_json::error::Error> for MatrixError {
    fn from(e: serde_json::error::Error) -> Self {
        Self::bad_json(e.to_string())
    }
}

impl Scribe for MatrixError {
    fn render(self, res: &mut Response) {
        res.add_header(header::CONTENT_TYPE, "application/json", true)
            .ok();

        if res.status_code.map(|c| c.is_success()).unwrap_or(true) {
            let code = self.status_code.unwrap_or_else(|| {
                use ErrorKind::*;
                match self.kind.clone() {
                    Forbidden { .. }
                    | GuestAccessForbidden
                    | ThreepidAuthFailed
                    | ThreepidDenied => StatusCode::FORBIDDEN,
                    Unauthorized | UnknownToken { .. } | MissingToken => StatusCode::UNAUTHORIZED,
                    NotFound | Unrecognized => StatusCode::NOT_FOUND,
                    LimitExceeded { .. } => StatusCode::TOO_MANY_REQUESTS,
                    UserDeactivated => StatusCode::FORBIDDEN,
                    TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
                    CannotOverwriteMedia => StatusCode::CONFLICT,
                    NotYetUploaded => StatusCode::GATEWAY_TIMEOUT,
                    _ => StatusCode::BAD_REQUEST,
                }
            });
            res.status_code(code);
        }

        if let Some(auth_error) = &self.authenticate {
            res.add_header(header::WWW_AUTHENTICATE, auth_error, true)
                .ok();
        };

        let Self { kind, mut body, .. } = self;
        body.0
            .insert("errcode".to_owned(), kind.code().to_string().into());

        let bytes: Vec<u8> = crate::serde::json_to_buf(&body.0).unwrap();
        res.write_body(bytes).ok();
    }
}

/// An error that happens when Palpo cannot understand a Matrix version.
#[derive(Debug)]
pub struct UnknownVersionError;

impl fmt::Display for UnknownVersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "version string was unknown")
    }
}

impl StdError for UnknownVersionError {}

/// An error that happens when an incorrect amount of arguments have been passed to [`PathBuilder`]
/// parts formatting.
///
/// [`PathBuilder`]: super::path_builder::PathBuilder
#[derive(Debug)]
pub struct IncorrectArgumentCount {
    /// The expected amount of arguments.
    pub expected: usize,

    /// The amount of arguments received.
    pub got: usize,
}

impl fmt::Display for IncorrectArgumentCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "incorrect path argument count, expected {}, got {}",
            self.expected, self.got
        )
    }
}

impl StdError for IncorrectArgumentCount {}

/// An error when serializing the HTTP headers.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HeaderSerializationError {
    /// Failed to convert a header value to `http::header::HeaderValue`.
    #[error(transparent)]
    ToHeaderValue(#[from] http::header::InvalidHeaderValue),

    /// The `SystemTime` could not be converted to a HTTP date.
    ///
    /// This only happens if the `SystemTime` provided is too far in the past
    /// (before the Unix epoch) or the future (after the year 9999).
    #[error("invalid HTTP date")]
    InvalidHttpDate,
}

/// An error when converting one of ruma's endpoint-specific request or response
/// types to the corresponding http type.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IntoHttpError {
    /// Failed to add the authentication scheme to the request.
    #[error("failed to add authentication scheme: {0}")]
    Authentication(Box<dyn std::error::Error + Send + Sync + 'static>),

    /// Tried to create a request with an old enough version, for which no unstable endpoint
    /// exists.
    ///
    /// This is also a fallback error for if the version is too new for this endpoint.
    #[error(
        "endpoint was not supported by server-reported versions, \
         but no unstable path to fall back to was defined"
    )]
    NoUnstablePath,

    /// Tried to create a request with [`MatrixVersion`]s for all of which this endpoint was
    /// removed.
    #[error(
        "could not create any path variant for endpoint, as it was removed in version {}",
        .0.as_str().expect("no endpoint was removed in Matrix 1.0")
    )]
    EndpointRemoved(MatrixVersion),

    /// JSON serialization failed.
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),

    /// Query parameter serialization failed.
    #[error("query parameter serialization failed: {0}")]
    Query(#[from] serde_html_form::ser::Error),

    /// Header serialization failed.
    #[error("header serialization failed: {0}")]
    Header(#[from] HeaderSerializationError),

    /// HTTP request construction failed.
    #[error("HTTP request construction failed: {0}")]
    Http(#[from] http::Error),
}

impl From<http::header::InvalidHeaderValue> for IntoHttpError {
    fn from(value: http::header::InvalidHeaderValue) -> Self {
        Self::Header(value.into())
    }
}

/// An error when deserializing the HTTP headers.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HeaderDeserializationError {
    /// Failed to convert `http::header::HeaderValue` to `str`.
    #[error("{0}")]
    ToStrError(#[from] http::header::ToStrError),

    /// Failed to convert `http::header::HeaderValue` to an integer.
    #[error("{0}")]
    ParseIntError(#[from] ParseIntError),

    /// Failed to parse a HTTP date from a `http::header::Value`.
    #[error("failed to parse HTTP date")]
    InvalidHttpDate,

    /// The given required header is missing.
    #[error("missing header `{0}`")]
    MissingHeader(String),

    /// The given header failed to parse.
    #[error("invalid header: {0}")]
    InvalidHeader(Box<dyn std::error::Error + Send + Sync + 'static>),

    /// A header was received with a unexpected value.
    #[error(
        "The {header} header was received with an unexpected value, \
         expected {expected}, received {unexpected}"
    )]
    InvalidHeaderValue {
        /// The name of the header containing the invalid value.
        header: String,
        /// The value the header should have been set to.
        expected: String,
        /// The value we instead received and rejected.
        unexpected: String,
    },

    /// The `Content-Type` header for a `multipart/mixed` response is missing
    /// the `boundary` attribute.
    #[error(
        "The `Content-Type` header for a `multipart/mixed` response is missing the `boundary` attribute"
    )]
    MissingMultipartBoundary,
}

// #[cfg(test)]
// mod tests {
//     use assert_matches2::assert_matches;
//     use serde_json::{from_value as from_json_value, json};

//     use super::{ErrorKind, StandardErrorBody};

//     #[test]
//     fn deserialize_forbidden() {
//         let deserialized: StandardErrorBody = from_json_value(json!({
//             "errcode": "M_FORBIDDEN",
//             "error": "You are not authorized to ban users in this room.",
//         }))
//         .unwrap();

//         assert_eq!(deserialized.kind, ErrorKind::Forbidden);
//         assert_eq!(
//             deserialized.message,
//             "You are not authorized to ban users in this room."
//         );
//     }

//     #[test]
//     fn deserialize_wrong_room_key_version() {
//         let deserialized: StandardErrorBody = from_json_value(json!({
//             "current_version": "42",
//             "errcode": "M_WRONG_ROOM_KEYS_VERSION",
//             "error": "Wrong backup version."
//         }))
//         .expect("We should be able to deserialize a wrong room keys version
// error");

//         assert_matches!(deserialized.kind, ErrorKind::WrongRoomKeysVersion {
// current_version });         assert_eq!(current_version.as_deref(),
// Some("42"));         assert_eq!(deserialized.message, "Wrong backup
// version.");     }

//     #[test]
//     fn custom_authenticate_error_sanity() {
//         use super::AuthenticateError;

//         let s = "Bearer error=\"custom_error\", misc=\"some content\"";

//         let error = AuthenticateError::from_str(s).unwrap();
//         let error_header = http::HeaderValue::try_from(&error).unwrap();

//         assert_eq!(error_header.to_str().unwrap(), s);
//     }

//     #[test]
//     fn serialize_insufficient_scope() {
//         use super::AuthenticateError;

//         let error = AuthenticateError::InsufficientScope {
//             scope: "something_privileged".to_owned(),
//         };
//         let error_header = http::HeaderValue::try_from(&error).unwrap();

//         assert_eq!(
//             error_header.to_str().unwrap(),
//             "Bearer error=\"insufficient_scope\",
// scope=\"something_privileged\""         );
//     }

//     #[test]
//     fn deserialize_insufficient_scope() {
//         use super::{AuthenticateError, Error, ErrorBody};
//         use crate::api::EndpointError;

//         let response = http::Response::builder()
//             .header(
//                 http::header::WWW_AUTHENTICATE,
//                 "Bearer error=\"insufficient_scope\",
// scope=\"something_privileged\"",             )
//             .status(http::StatusCode::UNAUTHORIZED)
//             .body(
//                 serde_json::to_string(&json!({
//                     "errcode": "M_FORBIDDEN",
//                     "error": "Insufficient privilege",
//                 }))
//                 .unwrap(),
//             )
//             .unwrap();
//         let error = Error::from_http_response(response);

//         assert_eq!(error.status_code, http::StatusCode::UNAUTHORIZED);
//         assert_matches!(error.body, ErrorBody::Standard { kind, message });
//         assert_eq!(kind, ErrorKind::Forbidden);
//         assert_eq!(message, "Insufficient privilege");
//         assert_matches!(error.authenticate,
// Some(AuthenticateError::InsufficientScope { scope }));         assert_eq!
// (scope, "something_privileged");     }
// }
