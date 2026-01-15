use std::borrow::Cow;
use std::collections::btree_map::{BTreeMap, Entry};
use std::fmt;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{self, Serialize, SerializeMap, Serializer};
use serde_json::from_value as from_json_value;

use super::ErrorKind;
use crate::PrivOwnedStr;
use crate::client::http_header::{http_date_to_system_time, system_time_to_http_date};
use crate::error::{HeaderDeserializationError, HeaderSerializationError};
use crate::macros::StringEnum;

enum Field<'de> {
    ErrorCode,
    SoftLogout,
    RetryAfterMs,
    RoomVersion,
    AdminContact,
    Status,
    Body,
    CurrentVersion,
    Other(Cow<'de, str>),
}

impl<'de> Field<'de> {
    fn new(s: Cow<'de, str>) -> Field<'de> {
        match s.as_ref() {
            "errcode" => Self::ErrorCode,
            "soft_logout" => Self::SoftLogout,
            "retry_after_ms" => Self::RetryAfterMs,
            "room_version" => Self::RoomVersion,
            "admin_contact" => Self::AdminContact,
            "status" => Self::Status,
            "body" => Self::Body,
            "current_version" => Self::CurrentVersion,
            _ => Self::Other(s),
        }
    }
}

impl<'de> Deserialize<'de> for Field<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Field<'de>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Field<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any struct field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Field<'de>, E>
            where
                E: de::Error,
            {
                Ok(Field::new(Cow::Owned(value.to_owned())))
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Field<'de>, E>
            where
                E: de::Error,
            {
                Ok(Field::new(Cow::Borrowed(value)))
            }

            fn visit_string<E>(self, value: String) -> Result<Field<'de>, E>
            where
                E: de::Error,
            {
                Ok(Field::new(Cow::Owned(value)))
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct ErrorKindVisitor;

impl<'de> Visitor<'de> for ErrorKindVisitor {
    type Value = ErrorKind;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("enum ErrorKind")
    }

    fn visit_map<V>(self, mut map: V) -> Result<ErrorKind, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut errcode = None;
        let mut soft_logout = None;
        let mut retry_after_ms = None;
        let mut room_version = None;
        let mut admin_contact = None;
        let mut status = None;
        let mut body = None;
        let mut current_version = None;
        let mut extra = BTreeMap::new();

        macro_rules! set_field {
            (errcode) => {
                set_field!(@inner errcode)
            };
            ($field:ident) => {
                match errcode {
                    Some(set_field!(@variant_containing $field)) | None => {
                        set_field!(@inner $field)
                    }
                    // if we already know we're deserializing a different variant to the one
                    // containing this field, ignore its value.
                    Some(_) => {
                        let _ = map.next_value::<de::IgnoredAny>()?;
                    },
                }
            };
            (@variant_containing soft_logout) => { ErrorCode::UnknownToken };
            (@variant_containing retry_after_ms) => { ErrorCode::LimitExceeded };
            (@variant_containing room_version) => { ErrorCode::IncompatibleRoomVersion };
            (@variant_containing admin_contact) => { ErrorCode::ResourceLimitExceeded };
            (@variant_containing status) => { ErrorCode::BadStatus };
            (@variant_containing body) => { ErrorCode::BadStatus };
            (@variant_containing current_version) => { ErrorCode::WrongRoomKeysVersion };
            (@inner $field:ident) => {
                {
                    if $field.is_some() {
                        return Err(de::Error::duplicate_field(stringify!($field)));
                    }
                    $field = Some(map.next_value()?);
                }
            };
        }

        while let Some(key) = map.next_key()? {
            match key {
                Field::ErrorCode => set_field!(errcode),
                Field::SoftLogout => set_field!(soft_logout),
                Field::RetryAfterMs => set_field!(retry_after_ms),
                Field::RoomVersion => set_field!(room_version),
                Field::AdminContact => set_field!(admin_contact),
                Field::Status => set_field!(status),
                Field::Body => set_field!(body),
                Field::CurrentVersion => set_field!(current_version),
                Field::Other(other) => match extra.entry(other.into_owned()) {
                    Entry::Vacant(v) => {
                        v.insert(map.next_value()?);
                    }
                    Entry::Occupied(o) => {
                        return Err(de::Error::custom(format!("duplicate field `{}`", o.key())));
                    }
                },
            }
        }

        let errcode = errcode.ok_or_else(|| de::Error::missing_field("errcode"))?;

        Ok(match errcode {
            ErrorCode::AppserviceLoginUnsupported => ErrorKind::AppserviceLoginUnsupported,
            ErrorCode::BadAlias => ErrorKind::BadAlias,
            ErrorCode::BadJson => ErrorKind::BadJson,
            ErrorCode::BadState => ErrorKind::BadState,
            ErrorCode::BadStatus => ErrorKind::BadStatus {
                status: status
                    .map(|s| {
                        from_json_value::<u16>(s)
                            .map_err(de::Error::custom)?
                            .try_into()
                            .map_err(de::Error::custom)
                    })
                    .transpose()?,
                body: body
                    .map(from_json_value)
                    .transpose()
                    .map_err(de::Error::custom)?,
            },
            ErrorCode::CannotLeaveServerNoticeRoom => ErrorKind::CannotLeaveServerNoticeRoom,
            ErrorCode::CannotOverwriteMedia => ErrorKind::CannotOverwriteMedia,
            ErrorCode::CaptchaInvalid => ErrorKind::CaptchaInvalid,
            ErrorCode::CaptchaNeeded => ErrorKind::CaptchaNeeded,
            #[cfg(feature = "unstable-msc4306")]
            ErrorCode::ConflictingUnsubscription => ErrorKind::ConflictingUnsubscription,
            ErrorCode::ConnectionFailed => ErrorKind::ConnectionFailed,
            ErrorCode::ConnectionTimeout => ErrorKind::ConnectionTimeout,
            ErrorCode::DuplicateAnnotation => ErrorKind::DuplicateAnnotation,
            ErrorCode::Exclusive => ErrorKind::Exclusive,
            ErrorCode::Forbidden => ErrorKind::forbidden(),
            ErrorCode::GuestAccessForbidden => ErrorKind::GuestAccessForbidden,
            ErrorCode::IncompatibleRoomVersion => ErrorKind::IncompatibleRoomVersion {
                room_version: from_json_value(
                    room_version.ok_or_else(|| de::Error::missing_field("room_version"))?,
                )
                .map_err(de::Error::custom)?,
            },
            ErrorCode::InvalidParam => ErrorKind::InvalidParam,
            ErrorCode::InvalidRoomState => ErrorKind::InvalidRoomState,
            ErrorCode::InvalidUsername => ErrorKind::InvalidUsername,
            #[cfg(feature = "unstable-msc4380")]
            ErrorCode::InviteBlocked => ErrorKind::InviteBlocked,
            ErrorCode::LimitExceeded => ErrorKind::LimitExceeded {
                retry_after: retry_after_ms
                    .map(from_json_value::<u64>)
                    .transpose()
                    .map_err(de::Error::custom)?
                    .map(Duration::from_millis)
                    .map(RetryAfter::Delay),
            },
            ErrorCode::MissingParam => ErrorKind::MissingParam,
            ErrorCode::MissingToken => ErrorKind::MissingToken,
            ErrorCode::NotFound => ErrorKind::NotFound,
            #[cfg(feature = "unstable-msc4306")]
            ErrorCode::NotInThread => ErrorKind::NotInThread,
            ErrorCode::NotJson => ErrorKind::NotJson,
            ErrorCode::NotYetUploaded => ErrorKind::NotYetUploaded,
            ErrorCode::ResourceLimitExceeded => ErrorKind::ResourceLimitExceeded {
                admin_contact: from_json_value(
                    admin_contact.ok_or_else(|| de::Error::missing_field("admin_contact"))?,
                )
                .map_err(de::Error::custom)?,
            },
            ErrorCode::RoomInUse => ErrorKind::RoomInUse,
            ErrorCode::ServerNotTrusted => ErrorKind::ServerNotTrusted,
            ErrorCode::ThreepidAuthFailed => ErrorKind::ThreepidAuthFailed,
            ErrorCode::ThreepidDenied => ErrorKind::ThreepidDenied,
            ErrorCode::ThreepidInUse => ErrorKind::ThreepidInUse,
            ErrorCode::ThreepidMediumNotSupported => ErrorKind::ThreepidMediumNotSupported,
            ErrorCode::ThreepidNotFound => ErrorKind::ThreepidNotFound,
            ErrorCode::TooLarge => ErrorKind::TooLarge,
            ErrorCode::UnableToAuthorizeJoin => ErrorKind::UnableToAuthorizeJoin,
            ErrorCode::UnableToGrantJoin => ErrorKind::UnableToGrantJoin,
            #[cfg(feature = "unstable-msc3843")]
            ErrorCode::Unactionable => ErrorKind::Unactionable,
            ErrorCode::Unauthorized => ErrorKind::Unauthorized,
            ErrorCode::Unknown => ErrorKind::Unknown,
            #[cfg(feature = "unstable-msc4186")]
            ErrorCode::UnknownPos => ErrorKind::UnknownPos,
            ErrorCode::UnknownToken => ErrorKind::UnknownToken {
                soft_logout: soft_logout
                    .map(from_json_value)
                    .transpose()
                    .map_err(de::Error::custom)?
                    .unwrap_or_default(),
            },
            ErrorCode::Unrecognized => ErrorKind::Unrecognized,
            ErrorCode::UnsupportedRoomVersion => ErrorKind::UnsupportedRoomVersion,
            ErrorCode::UrlNotSet => ErrorKind::UrlNotSet,
            ErrorCode::UserDeactivated => ErrorKind::UserDeactivated,
            ErrorCode::UserInUse => ErrorKind::UserInUse,
            ErrorCode::UserLocked => ErrorKind::UserLocked,
            ErrorCode::UserSuspended => ErrorKind::UserSuspended,
            ErrorCode::WeakPassword => ErrorKind::WeakPassword,
            ErrorCode::WrongRoomKeysVersion => ErrorKind::WrongRoomKeysVersion {
                current_version: from_json_value(
                    current_version.ok_or_else(|| de::Error::missing_field("current_version"))?,
                )
                .map_err(de::Error::custom)?,
            },
            ErrorCode::_Custom(errcode) => ErrorKind::_Custom { errcode, extra },
        })
    }
}

/// The possible [error codes] defined in the Matrix spec.
///
/// [error codes]: https://spec.matrix.org/latest/client-server-api/#standard-error-response
#[derive(StringEnum, Clone)]
#[palpo_enum(rename_all(prefix = "M_", rule = "SCREAMING_SNAKE_CASE"))]
// Please keep the variants sorted alphabetically.
pub enum ErrorCode {
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
    BadStatus,

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
    #[palpo_enum(rename = "IO.ELEMENT.MSC4306.M_CONFLICTING_UNSUBSCRIPTION")]
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
    IncompatibleRoomVersion,

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
    ///
    /// Unstable prefix intentionally shared with MSC4155 for compatibility.
    #[cfg(feature = "unstable-msc4380")]
    #[palpo_enum(rename = "ORG.MATRIX.MSC4155.INVITE_BLOCKED")]
    InviteBlocked,

    /// `M_LIMIT_EXCEEDED`
    ///
    /// The request has been refused due to [rate limiting]: too many requests
    /// have been sent in a short period of time.
    ///
    /// [rate limiting]: https://spec.matrix.org/latest/client-server-api/#rate-limiting
    LimitExceeded,

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
    #[palpo_enum(rename = "IO.ELEMENT.MSC4306.M_NOT_IN_THREAD")]
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
    ResourceLimitExceeded,

    /// `M_ROOM_IN_USE`
    ///
    /// The [room alias] specified in the [`create_room`] request is already
    /// taken.
    ///
    /// [`create_room`]: crate::room::create_room
    /// [room alias]: https://spec.matrix.org/latest/client-server-api/#room-aliases
    RoomInUse,

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
    #[palpo_enum(rename = "M_UNABLE_TO_AUTHORISE_JOIN")]
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
    UnknownToken,

    /// `M_UNRECOGNIZED`
    ///
    /// The server did not understand the request.
    ///
    /// This is expected to be returned with a 404 HTTP status code if the
    /// endpoint is not implemented or a 405 HTTP status code if the
    /// endpoint is implemented, but the incorrect HTTP method is used.
    Unrecognized,

    /// `M_UNSUPPORTED_ROOM_VERSION`
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
    WrongRoomKeysVersion,

    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

impl<'de> Deserialize<'de> for ErrorKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ErrorKindVisitor)
    }
}

impl Serialize for ErrorKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut st = serializer.serialize_map(None)?;
        st.serialize_entry("errcode", &self.code())?;
        match self {
            Self::UnknownToken { soft_logout: true } => {
                st.serialize_entry("soft_logout", &true)?;
            }
            Self::LimitExceeded {
                retry_after: Some(RetryAfter::Delay(duration)),
            } => {
                st.serialize_entry(
                    "retry_after_ms",
                    &u64::try_from(duration.as_millis()).map_err(ser::Error::custom)?,
                )?;
            }
            Self::IncompatibleRoomVersion { room_version } => {
                st.serialize_entry("room_version", room_version)?;
            }
            Self::ResourceLimitExceeded { admin_contact } => {
                st.serialize_entry("admin_contact", admin_contact)?;
            }
            Self::_Custom { extra, .. } => {
                for (k, v) in extra {
                    st.serialize_entry(k, v)?;
                }
            }
            _ => {}
        }
        st.end()
    }
}

/// How long a client should wait before it tries again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::exhaustive_enums)]
pub enum RetryAfter {
    /// The client should wait for the given duration.
    ///
    /// This variant should be preferred for backwards compatibility, as it will
    /// also populate the `retry_after_ms` field in the body of the
    /// response.
    Delay(Duration),
    /// The client should wait for the given date and time.
    DateTime(SystemTime),
}

impl TryFrom<&http::HeaderValue> for RetryAfter {
    type Error = HeaderDeserializationError;

    fn try_from(value: &http::HeaderValue) -> Result<Self, Self::Error> {
        if value.as_bytes().iter().all(|b| b.is_ascii_digit()) {
            // It should be a duration.
            Ok(Self::Delay(Duration::from_secs(u64::from_str(
                value.to_str()?,
            )?)))
        } else {
            // It should be a date.
            Ok(Self::DateTime(http_date_to_system_time(value)?))
        }
    }
}

impl TryFrom<&RetryAfter> for http::HeaderValue {
    type Error = HeaderSerializationError;

    fn try_from(value: &RetryAfter) -> Result<Self, Self::Error> {
        match value {
            RetryAfter::Delay(duration) => Ok(duration.as_secs().into()),
            RetryAfter::DateTime(time) => system_time_to_http_date(time),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json};

    use super::ErrorKind;
    use crate::room_version_id;

    // #[test]
    // fn deserialize_forbidden() {
    //     let deserialized: ErrorKind = from_json_value(json!({ "errcode": "M_FORBIDDEN"
    // })).unwrap();     assert_eq!(deserialized, ErrorKind::Forbidden);
    // }

    // #[test]
    // fn deserialize_forbidden_with_extra_fields() {
    //     let deserialized: ErrorKind = from_json_value(json!({
    //         "errcode": "M_FORBIDDEN",
    //         "error": "…",
    //     }))
    //     .unwrap();

    //     assert_eq!(deserialized, ErrorKind::Forbidden);
    // }

    #[test]
    fn deserialize_incompatible_room_version() {
        let deserialized: ErrorKind = from_json_value(json!({
            "errcode": "M_INCOMPATIBLE_ROOM_VERSION",
            "room_version": "7",
        }))
        .unwrap();

        assert_eq!(
            deserialized,
            ErrorKind::IncompatibleRoomVersion {
                room_version: room_version_id!("7")
            }
        );
    }
}
