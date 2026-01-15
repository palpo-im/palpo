//! `POST /_matrix/client/*/register`
//!
//! Register an account on this homeserver.
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3register

use std::time::Duration;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::client::account::{LoginType, RegistrationKind};
use crate::client::uiaa::AuthData;
use crate::{OwnedClientSecret, OwnedDeviceId, OwnedSessionId, OwnedUserId};

/// Request type for the `register` endpoint.
#[derive(ToSchema, Deserialize, Default, Debug)]
pub struct RegisterReqBody {
    /// The desired password for the account.
    ///
    /// May be empty for accounts that should not be able to log in again
    /// with a password, e.g., for guest or application service accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,

    /// Localpart of the desired Matrix ID.
    ///
    /// If omitted, the homeserver MUST generate a Matrix ID local part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// ID of the client device.
    ///
    /// If this does not correspond to a known client device, a new device will
    /// be created. The server will auto-generate a device_id if this is not
    /// specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<OwnedDeviceId>,

    /// A display name to assign to the newly-created device.
    ///
    /// Ignored if `device_id` corresponds to a known device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_device_display_name: Option<String>,

    /// Additional authentication information for the user-interactive
    /// authentication API.
    ///
    /// Note that this information is not used to define how the registered user
    /// should be authenticated, but is instead used to authenticate the
    /// register call itself. It should be left empty, or omitted, unless an
    /// earlier call returned an response with status code 401.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,

    /// Kind of account to register
    ///
    /// Defaults to `User` if omitted.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub kind: RegistrationKind,

    /// If `true`, an `access_token` and `device_id` should not be returned
    /// from this call, therefore preventing an automatic login.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub inhibit_login: bool,

    /// Login `type` used by Appservices.
    ///
    /// Appservices can [bypass the registration flows][admin] entirely by
    /// providing their token in the header and setting this login `type` to
    /// `m.login.application_service`.
    ///
    /// [admin]: https://spec.matrix.org/latest/application-service-api/#server-admin-style-permissions
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub login_type: Option<LoginType>,

    /// If set to `true`, the client supports [refresh tokens].
    ///
    /// [refresh tokens]: https://spec.matrix.org/latest/client-server-api/#refreshing-access-tokens
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub refresh_token: bool,
}
impl RegisterReqBody {
    pub fn is_default(&self) -> bool {
        self.password.is_none()
            && self.username.is_none()
            && self.device_id.is_none()
            && self.initial_device_display_name.is_none()
            && self.auth.is_none()
            && self.kind == Default::default()
            && !self.inhibit_login
            && self.login_type.is_none()
            && !self.refresh_token
    }
}

/// Response type for the `register` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct RegisterResBody {
    /// An access token for the account.
    ///
    /// This access token can then be used to authorize other requests.
    ///
    /// Required if the request's `inhibit_login` was set to `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,

    /// The fully-qualified Matrix ID that has been registered.
    pub user_id: OwnedUserId,

    /// ID of the registered device.
    ///
    /// Will be the same as the corresponding parameter in the request, if one
    /// was specified.
    ///
    /// Required if the request's `inhibit_login` was set to `false`.
    pub device_id: Option<OwnedDeviceId>,

    /// A [refresh token] for the account.
    ///
    /// This token can be used to obtain a new access token when it expires by
    /// calling the [`refresh_token`] endpoint.
    ///
    /// Omitted if the request's `inhibit_login` was set to `true`.
    ///
    /// [refresh token]: https://spec.matrix.org/latest/client-server-api/#refreshing-access-tokens
    /// [`refresh_token`]: crate::session::refresh_token
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// The lifetime of the access token, in milliseconds.
    ///
    /// Once the access token has expired, a new access token can be obtained by
    /// using the provided refresh token. If no refresh token is provided,
    /// the client will need to re-login to obtain a new access token.
    ///
    /// If this is `None`, the client can assume that the access token will not
    /// expire.
    ///
    /// Omitted if the request's `inhibit_login` was set to `true`.
    #[serde(
        with = "palpo_core::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none",
        rename = "expires_in_ms"
    )]
    pub expires_in: Option<Duration>,
}

// /// `GET /_matrix/client/*/register/available`
// ///        1.0 => "/_matrix/client/r0/register/available",
// ///        1.1 => "/_matrix/client/v3/register/available",
// ///
// /// Checks to see if a username is available, and valid, for the server.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3registeravailable

/// Response type for the `get_username_availability` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct AvailableResBody {
    /// A flag to indicate that the username is available.
    /// This should always be true when the server replies with 200 OK.
    pub available: bool,
}
impl AvailableResBody {
    /// Creates a new `AvailableResBody` with the given availability.
    pub fn new(available: bool) -> Self {
        Self { available }
    }
}

// /// `GET /_matrix/client/*/register/m.login.registration_token/validity`
// ///
// /// Checks to see if the given registration token is valid.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1registermloginregistration_tokenvalidity

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/org.matrix.msc3231/register/org.matrix.msc3231.login.
// registration_token/validity",         1.2 =>
// "/_matrix/client/v1/register/m.login.registration_token/validity",     }
// };

/// Request type for the `check_registration_token_validity` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct ValidateTokenReqBody {
    /// The registration token to check the validity of.
    pub registration_token: String,
}

/// Response type for the `check_registration_token_validity` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct ValidateTokenResBody {
    /// A flag to indicate that the registration token is valid.
    pub valid: bool,
}

// `POST /_matrix/client/*/register/email/requestToken`
// /// Request a registration token with a 3rd party email.
// ///
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3registeremailrequesttoken

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/register/email/requestToken",
//         1.1 => "/_matrix/client/v3/register/email/requestToken",
//     }
// };

/// Request type for the `request_registration_token_via_email` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct TokenVisEmailReqBody {
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

/// Response type for the `request_registration_token_via_email` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct TokenVisEmailResBody {
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

// /// `POST /_matrix/client/*/register/msisdn/requestToken`
// ///
// /// Request a registration token with a phone number.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3registermsisdnrequesttoken

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/register/msisdn/requestToken",
//         1.1 => "/_matrix/client/v3/register/msisdn/requestToken",
//     }
// };

/// Request type for the `request_registration_token_via_msisdn` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct TokenVisMsisdnReqBody {
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

/// Response type for the `request_registration_token_via_msisdn` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct TokenVisMsisdnResBody {
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
