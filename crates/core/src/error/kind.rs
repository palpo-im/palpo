//! Errors that can be sent from the homeserver.

use std::collections::BTreeMap;

use serde_json::Value as JsonValue;

use super::{ErrorCode, RetryAfter};
use crate::{OwnedUserId, PrivOwnedStr, RoomVersionId};

/// An enum for the error kind.
///
/// Items may contain additional information.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// Please keep the variants sorted alphabetically.
pub enum ErrorKind {
    /// `M_APPSERVICE_LOGIN_UNSUPPORTED`
    ///
    /// An application service used the [`m.login.application_service`] type an endpoint from the
    /// [legacy authentication API] in a way that is not supported by the homeserver, because the
    /// server only supports the [OAuth 2.0 API].
    ///
    /// [`m.login.application_service`]: https://spec.matrix.org/latest/application-service-api/#server-admin-style-permissions
    /// [legacy authentication API]: https://spec.matrix.org/latest/client-server-api/#legacy-api
    /// [OAuth 2.0 API]: https://spec.matrix.org/latest/client-server-api/#oauth-20-api
    AppserviceLoginUnsupported,

    /// `M_BAD_ALIAS`
    ///
    /// One or more [room aliases] within the `m.room.canonical_alias` event do
    /// not point to the room ID for which the state event is to be sent to.
    ///
    /// [room aliases]: https://spec.matrix.org/latest/client-server-api/#room-aliases
    BadAlias,

    /// `M_BAD_JSON`
    ///
    /// The request contained valid JSON, but it was malformed in some way, e.g.
    /// missing required keys, invalid values for keys.
    BadJson,

    /// `M_BAD_STATE`
    ///
    /// The state change requested cannot be performed, such as attempting to
    /// unban a user who is not banned.
    BadState,

    /// `M_BAD_STATUS`
    ///
    /// The application service returned a bad status.
    BadStatus {
        /// The HTTP status code of the response.
        status: Option<http::StatusCode>,

        /// The body of the response.
        body: Option<String>,
    },

    /// `M_CANNOT_LEAVE_SERVER_NOTICE_ROOM`
    ///
    /// The user is unable to reject an invite to join the [server notices]
    /// room.
    ///
    /// [server notices]: https://spec.matrix.org/latest/client-server-api/#server-notices
    CannotLeaveServerNoticeRoom,

    /// `M_CANNOT_OVERWRITE_MEDIA`
    ///
    /// The [`create_content_async`] endpoint was called with a media ID that
    /// already has content.
    ///
    /// [`create_content_async`]: crate::media::create_content_async
    CannotOverwriteMedia,

    /// `M_CAPTCHA_INVALID`
    ///
    /// The Captcha provided did not match what was expected.
    CaptchaInvalid,

    /// `M_CAPTCHA_NEEDED`
    ///
    /// A Captcha is required to complete the request.
    CaptchaNeeded,

    /// `M_CONFLICTING_UNSUBSCRIPTION`
    ///
    /// Part of [MSC4306]: an automatic thread subscription has been skipped by the server, because
    /// the user unsubsubscribed after the indicated subscribed-to event.
    ///
    /// [MSC4306]: https://github.com/matrix-org/matrix-spec-proposals/pull/4306
    #[cfg(feature = "unstable-msc4306")]
    ConflictingUnsubscription,

    /// `M_CONNECTION_FAILED`
    ///
    /// The connection to the application service failed.
    ConnectionFailed,

    /// `M_CONNECTION_TIMEOUT`
    ///
    /// The connection to the application service timed out.
    ConnectionTimeout,

    /// `M_DUPLICATE_ANNOTATION`
    ///
    /// The request is an attempt to send a [duplicate annotation].
    ///
    /// [duplicate annotation]: https://spec.matrix.org/latest/client-server-api/#avoiding-duplicate-annotations
    DuplicateAnnotation,

    /// `M_EXCLUSIVE`
    ///
    /// The resource being requested is reserved by an application service, or
    /// the application service making the request has not created the
    /// resource.
    Exclusive,

    /// `M_FORBIDDEN`
    ///
    /// Forbidden access, e.g. joining a room without permission, failed login.
    Forbidden,

    /// `M_GUEST_ACCESS_FORBIDDEN`
    ///
    /// The room or resource does not permit [guests] to access it.
    ///
    /// [guests]: https://spec.matrix.org/latest/client-server-api/#guest-access
    GuestAccessForbidden,

    /// `M_INCOMPATIBLE_ROOM_VERSION`
    ///
    /// The client attempted to join a room that has a version the server does
    /// not support.
    IncompatibleRoomVersion {
        /// The room's version.
        room_version: RoomVersionId,
    },

    /// `M_INVALID_PARAM`
    ///
    /// A parameter that was specified has the wrong value. For example, the
    /// server expected an integer and instead received a string.
    InvalidParam,

    /// `M_INVALID_ROOM_STATE`
    ///
    /// The initial state implied by the parameters to the [`create_room`]
    /// request is invalid, e.g. the user's `power_level` is set below that
    /// necessary to set the room name.
    ///
    /// [`create_room`]: crate::room::create_room
    InvalidRoomState,

    /// `M_INVALID_USERNAME`
    ///
    /// The desired user name is not valid.
    InvalidUsername,

    /// `M_INVITE_BLOCKED`
    ///
    /// The invite was interdicted by moderation tools or configured access controls without having
    /// been witnessed by the invitee.
    InviteBlocked,

    /// `M_LIMIT_EXCEEDED`
    ///
    /// The request has been refused due to [rate limiting]: too many requests
    /// have been sent in a short period of time.
    ///
    /// [rate limiting]: https://spec.matrix.org/latest/client-server-api/#rate-limiting
    LimitExceeded {
        /// How long a client should wait before they can try again.
        retry_after: Option<RetryAfter>,
    },

    /// `M_MISSING_PARAM`
    ///
    /// A required parameter was missing from the request.
    MissingParam,

    /// `M_MISSING_TOKEN`
    ///
    /// No [access token] was specified for the request, but one is required.
    ///
    /// [access token]: https://spec.matrix.org/latest/client-server-api/#client-authentication
    MissingToken,

    /// `M_NOT_FOUND`
    ///
    /// No resource was found for this request.
    NotFound,

    /// `M_NOT_IN_THREAD`
    ///
    /// Part of [MSC4306]: an automatic thread subscription was set to an event ID that isn't part
    /// of the subscribed-to thread.
    ///
    /// [MSC4306]: https://github.com/matrix-org/matrix-spec-proposals/pull/4306
    #[cfg(feature = "unstable-msc4306")]
    NotInThread,

    /// `M_NOT_JSON`
    ///
    /// The request did not contain valid JSON.
    NotJson,

    /// `M_NOT_YET_UPLOADED`
    ///
    /// An `mxc:` URI generated with the [`create_mxc_uri`] endpoint was used
    /// and the content is not yet available.
    ///
    /// [`create_mxc_uri`]: crate::media::create_mxc_uri
    NotYetUploaded,

    /// `M_RESOURCE_LIMIT_EXCEEDED`
    ///
    /// The request cannot be completed because the homeserver has reached a
    /// resource limit imposed on it. For example, a homeserver held in a
    /// shared hosting environment may reach a resource limit if it starts
    /// using too much memory or disk space.
    ResourceLimitExceeded {
        /// A URI giving a contact method for the server administrator.
        admin_contact: String,
    },

    /// `M_ROOM_IN_USE`
    ///
    /// The [room alias] specified in the [`create_room`] request is already
    /// taken.
    ///
    /// [`create_room`]: crate::room::create_room
    /// [room alias]: https://spec.matrix.org/latest/client-server-api/#room-aliases
    RoomInUse,

    /// `UK.TIMEDOUT.MSC4406.SENDER_IGNORED`
    ///
    /// The sender of the requested event is ignored by the requesting user. ([MSC])
    ///
    /// [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4406
    SenderIgnored {
        /// The user who sent the ignored event.
        sender: Option<OwnedUserId>,
    },

    /// `M_SERVER_NOT_TRUSTED`
    ///
    /// The client's request used a third-party server, e.g. identity server,
    /// that this server does not trust.
    ServerNotTrusted,

    /// `M_THREEPID_AUTH_FAILED`
    ///
    /// Authentication could not be performed on the [third-party identifier].
    ///
    /// [third-party identifier]: https://spec.matrix.org/latest/client-server-api/#adding-account-administrative-contact-information
    ThreepidAuthFailed,

    /// `M_THREEPID_DENIED`
    ///
    /// The server does not permit this [third-party identifier]. This may
    /// happen if the server only permits, for example, email addresses from
    /// a particular domain.
    ///
    /// [third-party identifier]: https://spec.matrix.org/latest/client-server-api/#adding-account-administrative-contact-information
    ThreepidDenied,

    /// `M_THREEPID_IN_USE`
    ///
    /// The [third-party identifier] is already in use by another user.
    ///
    /// [third-party identifier]: https://spec.matrix.org/latest/client-server-api/#adding-account-administrative-contact-information
    ThreepidInUse,

    /// `M_THREEPID_MEDIUM_NOT_SUPPORTED`
    ///
    /// The homeserver does not support adding a [third-party identifier] of the
    /// given medium.
    ///
    /// [third-party identifier]: https://spec.matrix.org/latest/client-server-api/#adding-account-administrative-contact-information
    ThreepidMediumNotSupported,

    /// `M_THREEPID_NOT_FOUND`
    ///
    /// No account matching the given [third-party identifier] could be found.
    ///
    /// [third-party identifier]: https://spec.matrix.org/latest/client-server-api/#adding-account-administrative-contact-information
    ThreepidNotFound,

    /// `M_TOKEN_INCORRECT`
    ///
    /// The token that the user entered to validate the session is incorrect.
    TokenIncorrect,

    /// `M_TOO_LARGE`
    ///
    /// The request or entity was too large.
    TooLarge,

    /// `M_UNABLE_TO_AUTHORISE_JOIN`
    ///
    /// The room is [restricted] and none of the conditions can be validated by
    /// the homeserver. This can happen if the homeserver does not know
    /// about any of the rooms listed as conditions, for example.
    ///
    /// [restricted]: https://spec.matrix.org/latest/client-server-api/#restricted-rooms
    UnableToAuthorizeJoin,

    /// `M_UNABLE_TO_GRANT_JOIN`
    ///
    /// A different server should be attempted for the join. This is typically
    /// because the resident server can see that the joining user satisfies
    /// one or more conditions, such as in the case of [restricted rooms],
    /// but the resident server would be unable to meet the authorization
    /// rules.
    ///
    /// [restricted rooms]: https://spec.matrix.org/latest/client-server-api/#restricted-rooms
    UnableToGrantJoin,

    /// `M_UNACTIONABLE`
    ///
    /// The server does not want to handle the [federated report].
    ///
    /// [federated report]: https://github.com/matrix-org/matrix-spec-proposals/pull/3843
    #[cfg(feature = "unstable-msc3843")]
    Unactionable,

    /// `M_UNAUTHORIZED`
    ///
    /// The request was not correctly authorized. Usually due to login failures.
    Unauthorized,

    /// `M_UNKNOWN`
    ///
    /// An unknown error has occurred.
    Unknown,

    /// `M_UNKNOWN_POS`
    ///
    /// The sliding sync ([MSC4186]) connection was expired by the server.
    ///
    /// [MSC4186]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
    #[cfg(feature = "unstable-msc4186")]
    UnknownPos,

    /// `M_UNKNOWN_TOKEN`
    ///
    /// The [access or refresh token] specified was not recognized.
    ///
    /// [access or refresh token]: https://spec.matrix.org/latest/client-server-api/#client-authentication
    UnknownToken {
        /// If this is `true`, the client is in a "[soft logout]" state, i.e.
        /// the server requires re-authentication but the session is not
        /// invalidated. The client can acquire a new access token by
        /// specifying the device ID it is already using to the login API.
        ///
        /// [soft logout]: https://spec.matrix.org/latest/client-server-api/#soft-logout
        soft_logout: bool,
    },

    /// `M_UNRECOGNIZED`
    ///
    /// The server did not understand the request.
    ///
    /// This is expected to be returned with a 404 HTTP status code if the
    /// endpoint is not implemented or a 405 HTTP status code if the
    /// endpoint is implemented, but the incorrect HTTP method is used.
    Unrecognized,

    /// `M_UNSUPPORTED_ROOM_VERSION`
    ///
    /// The request to [`create_room`] used a room version that the server does
    /// not support.
    ///
    /// [`create_room`]: crate::room::create_room
    UnsupportedRoomVersion,

    /// `M_URL_NOT_SET`
    ///
    /// The application service doesn't have a URL configured.
    UrlNotSet,

    /// `M_USER_DEACTIVATED`
    ///
    /// The user ID associated with the request has been deactivated.
    UserDeactivated,

    /// `M_USER_IN_USE`
    ///
    /// The desired user ID is already taken.
    UserInUse,

    /// `M_USER_LOCKED`
    ///
    /// The account has been [locked] and cannot be used at this time.
    ///
    /// [locked]: https://spec.matrix.org/latest/client-server-api/#account-locking
    UserLocked,

    /// `M_USER_SUSPENDED`
    ///
    /// The account has been [suspended] and can only be used for limited
    /// actions at this time.
    ///
    /// [suspended]: https://spec.matrix.org/latest/client-server-api/#account-suspension
    UserSuspended,

    /// `M_WEAK_PASSWORD`
    ///
    /// The password was [rejected] by the server for being too weak.
    ///
    /// [rejected]: https://spec.matrix.org/latest/client-server-api/#notes-on-password-management
    WeakPassword,

    /// `M_WRONG_ROOM_KEYS_VERSION`
    ///
    /// The version of the [room keys backup] provided in the request does not
    /// match the current backup version.
    ///
    /// [room keys backup]: https://spec.matrix.org/latest/client-server-api/#server-side-key-backups
    WrongRoomKeysVersion {
        /// The currently active backup version.
        current_version: Option<String>,
    },

    #[doc(hidden)]
    _Custom {
        errcode: PrivOwnedStr,
        extra: BTreeMap<String, JsonValue>,
    },
}

impl ErrorKind {
    /// Get the [`ErrorCode`] for this `ErrorKind`.
    pub fn code(&self) -> ErrorCode {
        match self {
            ErrorKind::AppserviceLoginUnsupported => ErrorCode::AppserviceLoginUnsupported,
            ErrorKind::BadAlias => ErrorCode::BadAlias,
            ErrorKind::BadJson => ErrorCode::BadJson,
            ErrorKind::BadState => ErrorCode::BadState,
            ErrorKind::BadStatus { .. } => ErrorCode::BadStatus,
            ErrorKind::CannotLeaveServerNoticeRoom => ErrorCode::CannotLeaveServerNoticeRoom,
            ErrorKind::CannotOverwriteMedia => ErrorCode::CannotOverwriteMedia,
            ErrorKind::CaptchaInvalid => ErrorCode::CaptchaInvalid,
            ErrorKind::CaptchaNeeded => ErrorCode::CaptchaNeeded,
            #[cfg(feature = "unstable-msc4306")]
            ErrorKind::ConflictingUnsubscription => ErrorCode::ConflictingUnsubscription,
            ErrorKind::ConnectionFailed => ErrorCode::ConnectionFailed,
            ErrorKind::ConnectionTimeout => ErrorCode::ConnectionTimeout,
            ErrorKind::DuplicateAnnotation => ErrorCode::DuplicateAnnotation,
            ErrorKind::Exclusive => ErrorCode::Exclusive,
            ErrorKind::Forbidden => ErrorCode::Forbidden,
            ErrorKind::GuestAccessForbidden => ErrorCode::GuestAccessForbidden,
            ErrorKind::IncompatibleRoomVersion { .. } => ErrorCode::IncompatibleRoomVersion,
            ErrorKind::InvalidParam => ErrorCode::InvalidParam,
            ErrorKind::InvalidRoomState => ErrorCode::InvalidRoomState,
            ErrorKind::InvalidUsername => ErrorCode::InvalidUsername,
            ErrorKind::InviteBlocked => ErrorCode::InviteBlocked,
            ErrorKind::LimitExceeded { .. } => ErrorCode::LimitExceeded,
            ErrorKind::MissingParam => ErrorCode::MissingParam,
            ErrorKind::MissingToken => ErrorCode::MissingToken,
            ErrorKind::NotFound => ErrorCode::NotFound,
            #[cfg(feature = "unstable-msc4306")]
            ErrorKind::NotInThread => ErrorCode::NotInThread,
            ErrorKind::NotJson => ErrorCode::NotJson,
            ErrorKind::NotYetUploaded => ErrorCode::NotYetUploaded,
            ErrorKind::ResourceLimitExceeded { .. } => ErrorCode::ResourceLimitExceeded,
            ErrorKind::RoomInUse => ErrorCode::RoomInUse,
            ErrorKind::SenderIgnored { .. } => ErrorCode::SenderIgnored,
            ErrorKind::ServerNotTrusted => ErrorCode::ServerNotTrusted,
            ErrorKind::ThreepidAuthFailed => ErrorCode::ThreepidAuthFailed,
            ErrorKind::ThreepidDenied => ErrorCode::ThreepidDenied,
            ErrorKind::ThreepidInUse => ErrorCode::ThreepidInUse,
            ErrorKind::ThreepidMediumNotSupported => ErrorCode::ThreepidMediumNotSupported,
            ErrorKind::ThreepidNotFound => ErrorCode::ThreepidNotFound,
            ErrorKind::TokenIncorrect => ErrorCode::TokenIncorrect,
            ErrorKind::TooLarge => ErrorCode::TooLarge,
            ErrorKind::UnableToAuthorizeJoin => ErrorCode::UnableToAuthorizeJoin,
            ErrorKind::UnableToGrantJoin => ErrorCode::UnableToGrantJoin,
            #[cfg(feature = "unstable-msc3843")]
            ErrorKind::Unactionable => ErrorCode::Unactionable,
            ErrorKind::Unauthorized => ErrorCode::Unauthorized,
            ErrorKind::Unknown => ErrorCode::Unknown,
            #[cfg(feature = "unstable-msc4186")]
            ErrorKind::UnknownPos => ErrorCode::UnknownPos,
            ErrorKind::UnknownToken { .. } => ErrorCode::UnknownToken,
            ErrorKind::Unrecognized => ErrorCode::Unrecognized,
            ErrorKind::UnsupportedRoomVersion => ErrorCode::UnsupportedRoomVersion,
            ErrorKind::UrlNotSet => ErrorCode::UrlNotSet,
            ErrorKind::UserDeactivated => ErrorCode::UserDeactivated,
            ErrorKind::UserInUse => ErrorCode::UserInUse,
            ErrorKind::UserLocked => ErrorCode::UserLocked,
            ErrorKind::UserSuspended => ErrorCode::UserSuspended,
            ErrorKind::WeakPassword => ErrorCode::WeakPassword,
            ErrorKind::WrongRoomKeysVersion { .. } => ErrorCode::WrongRoomKeysVersion,
            ErrorKind::_Custom { errcode, .. } => errcode.0.clone().into(),
        }
    }
}
