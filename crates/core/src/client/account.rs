pub mod data;
pub mod threepid;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::client::uiaa::AuthData;
use crate::macros::StringEnum;
use crate::{OwnedClientSecret, OwnedDeviceId, OwnedSessionId, OwnedUserId, PrivOwnedStr};

/// Additional authentication information for requestToken endpoints.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct IdentityServerInfo {
    /// The ID server to send the onward request to as a hostname with an
    /// appended colon and port number if the port is not the default.
    pub id_server: String,

    /// Access token previously registered with identity server.
    pub id_access_token: String,
}

impl IdentityServerInfo {
    /// Creates a new `IdentityServerInfo` with the given server name and access
    /// token.
    pub fn new(id_server: String, id_access_token: String) -> Self {
        Self {
            id_server,
            id_access_token,
        }
    }
}

/// Possible values for deleting or unbinding 3PIDs.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ThirdPartyIdRemovalStatus {
    /// Either the homeserver couldn't determine the right identity server to
    /// contact, or the identity server refused the operation.
    NoSupport,

    /// Success.
    Success,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// The kind of account being registered.
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationKind {
    /// A guest account
    ///
    /// These accounts may have limited permissions and may not be supported by
    /// all servers.
    Guest,

    /// A regular user account
    #[default]
    User,
}

/// The login type.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum LoginType {
    /// An appservice-specific login type
    #[serde(rename = "m.login.application_service")]
    Appservice,
}

/// WhoamiResBody type for the `whoami` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct WhoamiResBody {
    /// The id of the user that owns the access token.
    pub user_id: OwnedUserId,

    /// The device ID associated with the access token, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<OwnedDeviceId>,

    /// If `true`, the user is a guest user.
    #[serde(default, skip_serializing_if = "palpo_core::serde::is_default")]
    pub is_guest: bool,
}

impl WhoamiResBody {
    /// Creates a new `Response` with the given user ID.
    pub fn new(user_id: OwnedUserId, is_guest: bool) -> Self {
        Self {
            user_id,
            device_id: None,
            is_guest,
        }
    }
}

// `POST /_matrix/client/*/account/deactivate`
//
// Deactivate the current user's account.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3accountdeactivate

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/account/deactivate",
//         1.1 => "/_matrix/client/v3/account/deactivate",
//     }
// };

/// Request type for the `deactivate` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct DeactivateReqBody {
    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,

    /// Identity server from which to unbind the user's third party
    /// identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_server: Option<String>,

    /// Whether the user would like their content to be erased as much as
    /// possible from the server.
    ///
    /// Defaults to `false`.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub erase: bool,
}

/// Response type for the `deactivate` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct DeactivateResBody {
    /// Result of unbind operation.
    pub id_server_unbind_result: ThirdPartyIdRemovalStatus,
}
impl DeactivateResBody {
    /// Creates a new `Response` with the given unbind result.
    pub fn new(id_server_unbind_result: ThirdPartyIdRemovalStatus) -> Self {
        Self {
            id_server_unbind_result,
        }
    }
}

/// Request type for the `change_password` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct ChangePasswordReqBody {
    /// The new password for the account.
    pub new_password: String,

    /// True to revoke the user's other access tokens, and their associated
    /// devices if the request succeeds.
    ///
    /// Defaults to true.
    ///
    /// When false, the server can still take advantage of the soft logout
    /// method for the user's remaining devices.
    #[serde(
        default = "crate::serde::default_true",
        skip_serializing_if = "crate::serde::is_true"
    )]
    pub logout_devices: bool,

    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,
}

// `POST /_matrix/client/*/account/password/email/requestToken`
//
// Request that a password change token is sent to the given email address.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3accountpasswordemailrequesttoken

//         1.0 => "/_matrix/client/r0/account/password/email/requestToken",
//         1.1 => "/_matrix/client/v3/account/password/email/requestToken",
/// Request type for the `request_password_change_token_via_email` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct TokenViaEmailReqBody {
    /// Client-generated secret string used to protect this session.
    pub client_secret: OwnedClientSecret,

    /// The email address.
    pub email: String,

    /// Used to distinguish protocol level retries from requests to re-send the
    /// email.
    pub send_attempt: u64,

    /// Return URL for identity server to redirect the client back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_link: Option<String>,
}

/// Response type for the `request_password_change_token_via_email` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct TokenViaEmailResBody {
    /// The session identifier given by the identity server.
    pub sid: OwnedSessionId,

    /// URL to submit validation token to.
    ///
    /// If omitted, verification happens without client.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub submit_url: Option<String>,
}

// `POST /_matrix/client/*/account/password/msisdn/requestToken`
//
// Request that a password change token is sent to the given phone number.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3accountpasswordmsisdnrequesttoken

//         1.0 => "/_matrix/client/r0/account/password/msisdn/requestToken",
//         1.1 => "/_matrix/client/v3/account/password/msisdn/requestToken",

/// Request type for the `request_password_change_token_via_msisdn` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct TokenViaMsisdnReqBody {
    /// Client-generated secret string used to protect this session.
    pub client_secret: OwnedClientSecret,

    /// Two-letter ISO 3166 country code for the phone number.
    pub country: String,

    /// Phone number to validate.
    pub phone_number: String,

    /// Used to distinguish protocol level retries from requests to re-send the
    /// SMS.
    pub send_attempt: u64,

    /// Return URL for identity server to redirect the client back to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_link: Option<String>,
}

/// Response type for the `request_password_change_token_via_msisdn` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct TokenViaMsisdnResBody {
    /// The session identifier given by the identity server.
    pub sid: OwnedSessionId,

    /// URL to submit validation token to.
    ///
    /// If omitted, verification happens without client.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub submit_url: Option<String>,
}
