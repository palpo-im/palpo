use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use salvo::prelude::*;
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value as JsonValue;

use crate::client::uiaa::{AuthData, UserIdentifier};
use crate::serde::{JsonObject, StringEnum};
use crate::{OwnedDeviceId, OwnedMxcUri, OwnedUserId, PrivOwnedStr};

// /// `POST /_matrix/client/*/refresh`
// ///
// /// Refresh an access token.
// ///
// /// Clients should use the returned access token when making subsequent API
// /// calls, and store the returned refresh token (if given) in order to refresh
// /// the new access token when necessary.
// ///
// /// After an access token has been refreshed, a server can choose to invalidate
// /// the old access token immediately, or can choose not to, for example if the
// /// access token would expire soon anyways. Clients should not make any
// /// assumptions about the old access token still being valid, and should use the
// /// newly provided access token instead.
// ///
// /// The old refresh token remains valid until the new access token or refresh
// /// token is used, at which point the old refresh token is revoked.
// ///
// /// Note that this endpoint does not require authentication via an access token.
// /// Authentication is provided via the refresh token.
// ///
// /// Application Service identity assertion is disabled for this endpoint.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3refresh

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         unstable => "/_matrix/client/unstable/org.matrix.msc2918/refresh",
//         1.3 => "/_matrix/client/v3/refresh",
//     }
// };

/// Request type for the `refresh` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct RefreshTokenReqBody {
    /// The refresh token.
    pub refresh_token: String,
}

/// Response type for the `refresh` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct RefreshTokenResBody {
    /// The new access token to use.
    pub access_token: String,

    /// The new refresh token to use when the access token needs to be refreshed
    /// again.
    ///
    /// If this is `None`, the old refresh token can be re-used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// The lifetime of the access token, in milliseconds.
    ///
    /// If this is `None`, the client can assume that the access token will not
    /// expire.
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_in_ms: Option<Duration>,
}
impl RefreshTokenResBody {
    /// Creates a new `Response` with the given access token.
    pub fn new(access_token: String) -> Self {
        Self {
            access_token,
            refresh_token: None,
            expires_in_ms: None,
        }
    }
}

// /// `POST /_matrix/client/*/login`
// ///
// /// Login to the homeserver.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3login

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/login",
//         1.1 => "/_matrix/client/v3/login",
//     }
// };

/// Request type for the `login` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct LoginReqBody {
    /// The authentication mechanism.
    #[serde(flatten)]
    pub login_info: LoginInfo,

    /// ID of the client device
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<OwnedDeviceId>,

    /// A display name to assign to the newly-created device.
    ///
    /// Ignored if `device_id` corresponds to a known device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_device_display_name: Option<String>,

    /// If set to `true`, the client supports [refresh tokens].
    ///
    /// [refresh tokens]: https://spec.matrix.org/latest/client-server-api/#refreshing-access-tokens
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub refresh_token: bool,
}

/// Response type for the `login` endpoint.
#[derive(ToSchema, Serialize, Debug)]

pub struct LoginResBody {
    /// The fully-qualified Matrix ID that has been registered.
    pub user_id: OwnedUserId,

    /// An access token for the account.
    pub access_token: String,

    /// ID of the logged-in device.
    ///
    /// Will be the same as the corresponding parameter in the request, if one
    /// was specified.
    pub device_id: OwnedDeviceId,

    /// Client configuration provided by the server.
    ///
    /// If present, clients SHOULD use the provided object to reconfigure
    /// themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub well_known: Option<DiscoveryInfo>,

    /// A [refresh token] for the account.
    ///
    /// This token can be used to obtain a new access token when it expires by
    /// calling the [`refresh_token`] endpoint.
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
    #[serde(
        with = "crate::serde::duration::opt_ms",
        default,
        skip_serializing_if = "Option::is_none",
        rename = "expires_in_ms"
    )]
    pub expires_in: Option<Duration>,
}
impl LoginResBody {
    /// Creates a new `Response` with the given user ID, access token and device
    /// ID.
    pub fn new(user_id: OwnedUserId, access_token: String, device_id: OwnedDeviceId) -> Self {
        Self {
            user_id,
            access_token,
            device_id,
            well_known: None,
            refresh_token: None,
            expires_in: None,
        }
    }
}

/// The authentication mechanism.
#[derive(ToSchema, Clone, Serialize)]
#[serde(untagged)]
pub enum LoginInfo {
    /// An identifier and password are supplied to authenticate.
    Password(Password),

    /// Token-based login.
    Token(Token),

    /// JSON Web Token
    Jwt(Token),

    /// Application Service-specific login.
    Appservice(Appservice),

    #[doc(hidden)]
    _Custom(CustomLoginInfo),
}

impl LoginInfo {
    /// Creates a new `IncomingLoginInfo` with the given `login_type` string,
    /// session and data.
    ///
    /// Prefer to use the public variants of `IncomingLoginInfo` where possible;
    /// this constructor is meant be used for unsupported authentication
    /// mechanisms only and does not allow setting arbitrary data for
    /// supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `login_type` is known and serialization of
    /// `data` to the corresponding `IncomingLoginInfo` variant fails.
    pub fn new(login_type: &str, data: JsonObject) -> serde_json::Result<Self> {
        Ok(match login_type {
            "m.login.password" => Self::Password(serde_json::from_value(JsonValue::Object(data))?),
            "m.login.token" => Self::Token(serde_json::from_value(JsonValue::Object(data))?),
            "m.login.application_service" => {
                Self::Appservice(serde_json::from_value(JsonValue::Object(data))?)
            }
            _ => Self::_Custom(CustomLoginInfo {
                login_type: login_type.into(),
                extra: data,
            }),
        })
    }
}

impl fmt::Debug for LoginInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print `Password { .. }` instead of `Password(Password { .. })`
        match self {
            Self::Password(inner) => inner.fmt(f),
            Self::Token(inner) => inner.fmt(f),
            Self::Jwt(inner) => inner.fmt(f),
            Self::Appservice(inner) => inner.fmt(f),
            Self::_Custom(inner) => inner.fmt(f),
        }
    }
}

impl<'de> Deserialize<'de> for LoginInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        fn from_json_value<T: DeserializeOwned, E: de::Error>(val: JsonValue) -> Result<T, E> {
            serde_json::from_value(val).map_err(E::custom)
        }

        // FIXME: Would be better to use serde_json::value::RawValue, but that would
        // require implementing Deserialize manually for Request, bc.
        // `#[serde(flatten)]` breaks things.
        let json = JsonValue::deserialize(deserializer)?;

        let login_type = json["type"]
            .as_str()
            .ok_or_else(|| de::Error::missing_field("type"))?;
        match login_type {
            "m.login.password" => from_json_value(json).map(Self::Password),
            "m.login.token" => from_json_value(json).map(Self::Token),
            "org.matrix.login.jwt" => from_json_value(json).map(Self::Jwt),
            "m.login.application_service" => from_json_value(json).map(Self::Appservice),
            _ => from_json_value(json).map(Self::_Custom),
        }
    }
}

/// An identifier and password to supply as authentication.
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.password")]
pub struct Password {
    /// Identification information for the user.
    pub identifier: UserIdentifier,

    /// The password.
    pub password: String,
}

impl Password {
    /// Creates a new `Password` with the given identifier and password.
    pub fn new(identifier: UserIdentifier, password: String) -> Self {
        Self {
            identifier,
            password,
        }
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            identifier,
            password: _,
        } = self;
        f.debug_struct("Password")
            .field("identifier", identifier)
            .finish_non_exhaustive()
    }
}

/// A token to supply as authentication.
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.token")]
pub struct Token {
    /// The token.
    pub token: String,
}

impl Token {
    /// Creates a new `Token` with the given token.
    pub fn new(token: String) -> Self {
        Self { token }
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { token: _ } = self;
        f.debug_struct("Token").finish_non_exhaustive()
    }
}

/// An identifier to supply for Application Service authentication.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "type", rename = "m.login.application_service")]
pub struct Appservice {
    /// Identification information for the user.
    pub identifier: UserIdentifier,
}

impl Appservice {
    /// Creates a new `Appservice` with the given identifier.
    pub fn new(identifier: UserIdentifier) -> Self {
        Self { identifier }
    }
}

#[doc(hidden)]
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CustomLoginInfo {
    #[serde(rename = "type")]
    login_type: String,
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    extra: JsonObject,
}

impl fmt::Debug for CustomLoginInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            login_type,
            extra: _,
        } = self;
        f.debug_struct("CustomLoginInfo")
            .field("login_type", login_type)
            .finish_non_exhaustive()
    }
}

/// Client configuration provided by the server.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct DiscoveryInfo {
    /// Information about the homeserver to connect to.
    #[serde(rename = "m.homeserver")]
    pub homeserver: HomeServerInfo,

    /// Information about the identity server to connect to.
    #[serde(default, rename = "m.identity_server")]
    pub identity_server: Option<IdentityServerInfo>,
}

impl DiscoveryInfo {
    /// Create a new `DiscoveryInfo` with the given homeserver.
    pub fn new(homeserver: HomeServerInfo) -> Self {
        Self {
            homeserver,
            identity_server: None,
        }
    }
}

/// Information about the homeserver to connect to.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct HomeServerInfo {
    /// The base URL for the homeserver for client-server connections.
    pub base_url: String,
}

impl HomeServerInfo {
    /// Create a new `HomeServerInfo` with the given base url.
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

/// Information about the identity server to connect to.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct IdentityServerInfo {
    /// The base URL for the identity server for client-server connections.
    pub base_url: String,
}

impl IdentityServerInfo {
    /// Create a new `IdentityServerInfo` with the given base url.
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

// /// `GET /_matrix/client/*/login`
// ///
// /// Gets the homeserver's supported login types to authenticate users. Clients
// /// should pick one of these and supply it as the type when logging in.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3login

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/login",
//         1.1 => "/_matrix/client/v3/login",
//     }
// };

/// Response type for the `get_login_types` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct LoginTypesResBody {
    /// The homeserver's supported login types.
    pub flows: Vec<LoginType>,
}

impl LoginTypesResBody {
    /// Creates a new `Response` with the given login types.
    pub fn new(flows: Vec<LoginType>) -> Self {
        Self { flows }
    }
}

/// An authentication mechanism.
#[derive(ToSchema, Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum LoginType {
    /// A password is supplied to authenticate.
    Password(PasswordLoginType),

    /// Token-based login.
    Token(TokenLoginType),

    /// JSON Web Token type.
    Jwt(JwtLoginType),

    /// SSO-based login.
    Sso(SsoLoginType),

    /// Appsrvice login.
    Appservice(AppserviceLoginType),

    /// Custom login type.
    #[doc(hidden)]
    _Custom(Box<CustomLoginType>),
}

impl LoginType {
    pub fn password() -> Self {
        Self::Password(PasswordLoginType::new())
    }
    pub fn appservice() -> Self {
        Self::Appservice(AppserviceLoginType::new())
    }
    pub fn jwt() -> Self {
        Self::Jwt(JwtLoginType::new())
    }
    /// Creates a new `LoginType` with the given `login_type` string and data.
    ///
    /// Prefer to use the public variants of `LoginType` where possible; this
    /// constructor is meant be used for unsupported login types only and
    /// does not allow setting arbitrary data for supported ones.
    pub fn new(login_type: &str, data: JsonObject) -> serde_json::Result<Self> {
        fn from_json_object<T: DeserializeOwned>(obj: JsonObject) -> serde_json::Result<T> {
            serde_json::from_value(JsonValue::Object(obj))
        }

        Ok(match login_type {
            "m.login.password" => Self::Password(from_json_object(data)?),
            "m.login.token" => Self::Token(from_json_object(data)?),
            "org.matrix.login.jwt" => Self::Jwt(from_json_object(data)?),
            "m.login.sso" => Self::Sso(from_json_object(data)?),
            "m.login.application_service" => Self::Appservice(from_json_object(data)?),
            _ => Self::_Custom(Box::new(CustomLoginType {
                type_: login_type.to_owned(),
                data,
            })),
        })
    }

    /// Returns a reference to the `login_type` string.
    pub fn login_type(&self) -> &str {
        match self {
            Self::Password(_) => "m.login.password",
            Self::Token(_) => "m.login.token",
            Self::Jwt(_) => "org.matrix.login.jwt",
            Self::Sso(_) => "m.login.sso",
            Self::Appservice(_) => "m.login.application_service",
            Self::_Custom(c) => &c.type_,
        }
    }

    /// Returns the associated data.
    ///
    /// Prefer to use the public variants of `LoginType` where possible; this
    /// method is meant to be used for unsupported login types only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        fn serialize<T: Serialize>(obj: &T) -> JsonObject {
            match serde_json::to_value(obj).expect("login type serialization to succeed") {
                JsonValue::Object(obj) => obj,
                _ => panic!("all login types must serialize to objects"),
            }
        }

        match self {
            Self::Password(d) => Cow::Owned(serialize(d)),
            Self::Token(d) => Cow::Owned(serialize(d)),
            Self::Jwt(d) => Cow::Owned(serialize(d)),
            Self::Sso(d) => Cow::Owned(serialize(d)),
            Self::Appservice(d) => Cow::Owned(serialize(d)),
            Self::_Custom(c) => Cow::Borrowed(&c.data),
        }
    }
}

/// The payload for password login.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.password")]
pub struct PasswordLoginType {}

impl PasswordLoginType {
    /// Creates a new `PasswordLoginType`.
    pub fn new() -> Self {
        Self {}
    }
}

/// The payload for token-based login.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.token")]
pub struct TokenLoginType {
    /// Whether the homeserver supports the `POST /login/get_token` endpoint.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    pub get_login_token: bool,
}

impl TokenLoginType {
    /// Creates a new `TokenLoginType`.
    pub fn new() -> Self {
        Self {
            get_login_token: false,
        }
    }
}

/// The payload for JWT-based login.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "org.matrix.login.jwt")]
pub struct JwtLoginType {}

impl JwtLoginType {
    /// Creates a new `JwtLoginType`.
    pub fn new() -> Self {
        Self {}
    }
}

/// The payload for SSO login.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.sso")]
pub struct SsoLoginType {
    /// The identity provider choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_providers: Vec<IdentityProvider>,
}

impl SsoLoginType {
    /// Creates a new `SsoLoginType`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// An SSO login identity provider.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct IdentityProvider {
    /// The ID of the provider.
    pub id: String,

    /// The display name of the provider.
    pub name: String,

    /// The icon for the provider.
    pub icon: Option<OwnedMxcUri>,

    /// The brand identifier for the provider.
    pub brand: Option<IdentityProviderBrand>,
}

impl IdentityProvider {
    /// Creates an `IdentityProvider` with the given `id` and `name`.
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            icon: None,
            brand: None,
        }
    }
}

/// An SSO login identity provider brand identifier.
///
/// The predefined ones can be found in the matrix-spec-proposals repo in a
/// [separate document][matrix-spec-proposals].
///
/// [matrix-spec-proposals]: https://github.com/matrix-org/matrix-spec-proposals/blob/v1.1/informal/idp-brands.md
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
pub enum IdentityProviderBrand {
    /// The [Apple] brand.
    ///
    /// [Apple]: https://developer.apple.com/design/human-interface-guidelines/sign-in-with-apple/overview/buttons/
    #[palpo_enum(rename = "apple")]
    Apple,

    /// The [Facebook](https://developers.facebook.com/docs/facebook-login/web/login-button/) brand.
    #[palpo_enum(rename = "facebook")]
    Facebook,

    /// The [GitHub](https://github.com/logos) brand.
    #[palpo_enum(rename = "github")]
    GitHub,

    /// The [GitLab](https://about.gitlab.com/press/press-kit/) brand.
    #[palpo_enum(rename = "gitlab")]
    GitLab,

    /// The [Google](https://developers.google.com/identity/branding-guidelines) brand.
    #[palpo_enum(rename = "google")]
    Google,

    /// The [Twitter] brand.
    ///
    /// [Twitter]: https://developer.twitter.com/en/docs/authentication/guides/log-in-with-twitter#tab1
    #[palpo_enum(rename = "twitter")]
    Twitter,

    /// A custom brand.
    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// The payload for Application Service login.
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.application_service")]
pub struct AppserviceLoginType {}

impl AppserviceLoginType {
    /// Creates a new `AppserviceLoginType`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A custom login payload.
#[doc(hidden)]
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[allow(clippy::exhaustive_structs)]
pub struct CustomLoginType {
    /// A custom type
    ///
    /// This field is named `type_` instead of `type` because the latter is a
    /// reserved keyword in Rust.
    #[serde(rename = "type")]
    pub type_: String,

    /// Remaining type content
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub data: JsonObject,
}

// /// `GET /_matrix/client/*/login/get_token`
// ///
// /// Generate a single-use, time-limited, `m.login.token` token.
// /// `/v1/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv1loginget_token
// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/org.matrix.msc3882/login/get_token",
//         1.7 => "/_matrix/client/v1/login/get_token",
//     }
// };

/// Request type for the `login` endpoint.

#[derive(ToSchema, Deserialize, Debug)]
pub struct TokenReqBody {
    /// Additional authentication information for the user-interactive
    /// authentication API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthData>,
}

/// Response type for the `login` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct TokenResBody {
    /// The time remaining in milliseconds until the homeserver will no longer
    /// accept the token.
    ///
    /// 2 minutes is recommended as a default.
    #[serde(with = "crate::serde::duration::ms", rename = "expires_in_ms")]
    pub expires_in: Duration,

    /// The login token for the `m.login.token` login flow.
    pub login_token: String,
}
impl TokenResBody {
    /// Creates a new `Response` with the given expiration duration and login
    /// token.
    pub fn new(expires_in: Duration, login_token: String) -> Self {
        Self {
            expires_in,
            login_token,
        }
    }

    /// Creates a new `Response` with the default expiration duration and the
    /// given login token.
    pub fn with_default_expiration_duration(login_token: String) -> Self {
        Self::new(Self::default_expiration_duration(), login_token)
    }

    fn default_expiration_duration() -> Duration {
        // 2 minutes.
        Duration::from_secs(2 * 60)
    }
}

// #[cfg(test)]
// mod tests {
//     use assert_matches2::assert_matches;

//     use super::{LoginInfo, Token};
//     use crate::uiaa::UserIdentifier;
//     use assert_matches2::assert_matches;
//     use serde::{Deserialize, Serialize};
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value, Value as JsonValue};

//     use super::{IdentityProvider, IdentityProviderBrand, LoginType,
// SsoLoginType, TokenLoginType};

//     #[derive(Debug, Deserialize, Serialize)]
//     struct Wrapper {
//         flows: Vec<LoginType>,
//     }

//     #[test]
//     fn deserialize_password_login_type() {
//         let wrapper = from_json_value::<Wrapper>(json!({
//             "flows": [
//                 { "type": "m.login.password" }
//             ],
//         }))
//         .unwrap();
//         assert_eq!(wrapper.flows.len(), 1);
//         assert_matches!(&wrapper.flows[0], LoginType::Password(_));
//     }

//     #[test]
//     fn deserialize_custom_login_type() {
//         let wrapper = from_json_value::<Wrapper>(json!({
//             "flows": [
//                 {
//                     "type": "io.palpo.custom",
//                     "color": "green",
//                 }
//             ],
//         }))
//         .unwrap();
//         assert_eq!(wrapper.flows.len(), 1);
//         assert_matches!(&wrapper.flows[0], LoginType::_Custom(custom));
//         assert_eq!(custom.type_, "io.palpo.custom");
//         assert_eq!(custom.data.len(), 1);
//         assert_eq!(custom.data.get("color"),
// Some(&JsonValue::from("green")));     }

//     #[test]
//     fn deserialize_sso_login_type() {
//         let wrapper = from_json_value::<Wrapper>(json!({
//             "flows": [
//                 {
//                     "type": "m.login.sso",
//                     "identity_providers": [
//                         {
//                             "id": "oidc-gitlab",
//                             "name": "GitLab",
//                             "icon": "mxc://localhost/gitlab-icon",
//                             "brand": "gitlab"
//                         },
//                         {
//                             "id": "custom",
//                             "name": "Custom",
//                         }
//                     ]
//                 }
//             ],
//         }))
//         .unwrap();
//         assert_eq!(wrapper.flows.len(), 1);
//         let flow = &wrapper.flows[0];

//         assert_matches!(flow, LoginType::Sso(SsoLoginType {
// identity_providers }));         assert_eq!(identity_providers.len(), 2);

//         let provider = &identity_providers[0];
//         assert_eq!(provider.id, "oidc-gitlab");
//         assert_eq!(provider.name, "GitLab");
//         assert_eq!(provider.icon.as_deref(),
// Some(mxc_uri!("mxc://localhost/gitlab-icon")));         assert_eq!(provider.
// brand, Some(IdentityProviderBrand::GitLab));

//         let provider = &identity_providers[1];
//         assert_eq!(provider.id, "custom");
//         assert_eq!(provider.name, "Custom");
//         assert_eq!(provider.icon, None);
//         assert_eq!(provider.brand, None);
//     }

//     #[test]
//     fn serialize_sso_login_type() {
//         let wrapper = to_json_value(Wrapper {
//             flows: vec![
//                 LoginType::Token(TokenLoginType::new()),
//                 LoginType::Sso(SsoLoginType {
//                     identity_providers: vec![IdentityProvider {
//                         id: "oidc-github".into(),
//                         name: "GitHub".into(),
//                         icon: Some("mxc://localhost/github-icon".into()),
//                         brand: Some(IdentityProviderBrand::GitHub),
//                     }],
//                 }),
//             ],
//         })
//         .unwrap();

//         assert_eq!(
//             wrapper,
//             json!({
//                 "flows": [
//                     {
//                         "type": "m.login.token"
//                     },
//                     {
//                         "type": "m.login.sso",
//                         "identity_providers": [
//                             {
//                                 "id": "oidc-github",
//                                 "name": "GitHub",
//                                 "icon": "mxc://localhost/github-icon",
//                                 "brand": "github"
//                             },
//                         ]
//                     }
//                 ],
//             })
//         );
//     }

//     #[test]
//     fn deserialize_login_type() {
//         assert_matches!(
//             from_json_value(json!({
//                 "type": "m.login.password",
//                 "identifier": {
//                     "type": "m.id.user",
//                     "user": "cheeky_monkey"
//                 },
//                 "password": "ilovebananas"
//             }))
//             .unwrap(),
//             LoginInfo::Password(login)
//         );
//         assert_matches!(login.identifier,
// UserIdentifier::UserIdOrLocalpart(user));         assert_eq!(user,
// "cheeky_monkey");         assert_eq!(login.password, "ilovebananas");

//         assert_matches!(
//             from_json_value(json!({
//                 "type": "m.login.token",
//                 "token": "1234567890abcdef"
//             }))
//             .unwrap(),
//             LoginInfo::Token(Token { token })
//         );
//         assert_eq!(token, "1234567890abcdef");
//     }
// }
