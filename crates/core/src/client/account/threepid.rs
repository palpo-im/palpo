//! `POST /_matrix/client/*/account/3pid/add`
//!
//! Add contact information to a user's account

//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3account3pidadd

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

//         1.0 => "/_matrix/client/r0/account/3pid/add",
//         1.1 => "/_matrix/client/v3/account/3pid/add",
//         1.0 => "/_matrix/client/r0/account/3pid/bind",
//         1.1 => "/_matrix/client/v3/account/3pid/bind",
use crate::client::account::IdentityServerInfo;
use crate::client::account::ThirdPartyIdRemovalStatus;
use crate::client::uiaa::AuthData;
use crate::third_party::{Medium, ThirdPartyIdentifier};
use crate::{OwnedClientSecret, OwnedSessionId};

#[derive(ToSchema, Serialize, Debug)]
pub struct ThreepidsResBody {
    /// A list of third party identifiers the homeserver has associated with the
    /// user's account.
    ///
    /// If the `compat-get-3pids` feature is enabled, this field will always be
    /// serialized, even if its value is an empty list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub three_pids: Vec<ThirdPartyIdentifier>,
}
impl ThreepidsResBody {
    pub fn new(three_pids: Vec<ThirdPartyIdentifier>) -> Self {
        Self { three_pids }
    }
}

/// Request type for the `add_3pid` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct AddThreepidReqBody {
    /// Additional information for the User-Interactive Authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,

    /// Client-generated secret string used to protect this session.
    pub client_secret: OwnedClientSecret,

    /// The session identifier given by the identity server.
    pub sid: OwnedSessionId,
}

/// Request type for the `bind_3pid` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct BindThreepidReqBody {
    /// Client-generated secret string used to protect this session.
    pub client_secret: OwnedClientSecret,

    /// The ID server to send the onward request to as a hostname with an
    /// appended colon and port number if the port is not the default.
    #[serde(flatten)]
    pub identity_server_info: IdentityServerInfo,

    /// The session identifier given by the identity server.
    pub sid: OwnedSessionId,
}

/// Request type for the `bind_3pid` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct UnbindThreepidReqBody {
    /// Identity server to unbind from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_server: Option<String>,

    /// Medium of the 3PID to be removed.
    pub medium: Medium,

    /// Third-party address being removed.
    pub address: String,
}

#[derive(ToSchema, Serialize, Debug)]
pub struct UnbindThreepidResBody {
    /// Result of unbind operation.
    pub id_server_unbind_result: ThirdPartyIdRemovalStatus,
}

/// Request type for the `bind_3pid` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct DeleteThreepidReqBody {
    /// Identity server to delete from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_server: Option<String>,

    /// Medium of the 3PID to be removed.
    pub medium: Medium,

    /// Third-party address being removed.
    pub address: String,
}

#[derive(ToSchema, Serialize, Debug)]
pub struct DeleteThreepidResBody {
    /// Result of unbind operation.
    pub id_server_unbind_result: ThirdPartyIdRemovalStatus,
}

// /// `POST /_matrix/client/*/account/3pid/email/requestToken`
// ///
// /// Request a 3PID management token with a 3rd party email.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3account3pidemailrequesttoken
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/account/3pid/email/requestToken",
//         1.1 => "/_matrix/client/v3/account/3pid/email/requestToken",
//     }
// };

/// Request type for the `request_3pid_management_token_via_email` endpoint.
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

/// Response type for the `request_3pid_management_token_via_email` endpoint.

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

// /// `POST /_matrix/client/*/account/3pid/msisdn/requestToken`
// ///
// /// Request a 3PID management token with a phone number.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3account3pidmsisdnrequesttoken

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/account/3pid/msisdn/requestToken",
//         1.1 => "/_matrix/client/v3/account/3pid/msisdn/requestToken",
//     }
// };

/// Request type for the `request_3pid_management_token_via_msisdn` endpoint.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_link: Option<String>,
}

/// Response type for the `request_3pid_management_token_via_msisdn` endpoint.

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
