//! Authentication data types for the different [`AuthType`]s.

use std::borrow::Cow;
use std::fmt;

use salvo::oapi::ToSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

mod data_serde;

use super::AuthType;
use crate::serde::JsonObject;
use crate::third_party::Medium;
use crate::{OwnedClientSecret, OwnedSessionId, OwnedUserId, PrivOwnedStr};

/// Information for one authentication stage.
#[derive(ToSchema, Clone, Serialize)]
#[non_exhaustive]
#[serde(untagged)]
pub enum AuthData {
    /// Password-based authentication (`m.login.password`).
    Password(Password),

    /// Google ReCaptcha 2.0 authentication (`m.login.recaptcha`).
    ReCaptcha(ReCaptcha),

    /// Email-based authentication (`m.login.email.identity`).
    EmailIdentity(EmailIdentity),

    /// Phone number-based authentication (`m.login.msisdn`).
    Msisdn(Msisdn),

    /// Dummy authentication (`m.login.dummy`).
    Dummy(Dummy),

    /// Registration token-based authentication (`m.login.registration_token`).
    RegistrationToken(RegistrationToken),

    /// Fallback acknowledgement.
    FallbackAcknowledgement(FallbackAcknowledgement),

    /// Terms of service (`m.login.terms`).
    ///
    /// This type is only valid during account registration.
    Terms(Terms),

    /// OAuth 2.0 (`m.oauth`).
    ///
    /// This type is only valid with the cross-signing keys upload endpoint, after logging in with
    /// the OAuth 2.0 API.
    OAuth(OAuth),

    /// Unsupported authentication type.
    #[doc(hidden)]
    #[salvo(schema(value_type = Object))]
    _Custom(CustomAuthData),
}

impl AuthData {
    /// Creates a new `AuthData` with the given `auth_type` string, session and data.
    ///
    /// Prefer to use the public variants of `AuthData` where possible; this constructor is meant to
    /// be used for unsupported authentication types only and does not allow setting arbitrary
    /// data for supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `auth_type` is known and serialization of `data` to the
    /// corresponding `AuthData` variant fails.
    pub fn new(
        auth_type: &str,
        session: Option<String>,
        data: JsonObject,
    ) -> serde_json::Result<Self> {
        fn deserialize_variant<T: DeserializeOwned>(
            session: Option<String>,
            mut obj: JsonObject,
        ) -> serde_json::Result<T> {
            if let Some(session) = session {
                obj.insert("session".into(), session.into());
            }
            serde_json::from_value(JsonValue::Object(obj))
        }

        Ok(match auth_type {
            "m.login.password" => Self::Password(deserialize_variant(session, data)?),
            "m.login.recaptcha" => Self::ReCaptcha(deserialize_variant(session, data)?),
            "m.login.email.identity" => Self::EmailIdentity(deserialize_variant(session, data)?),
            "m.login.msisdn" => Self::Msisdn(deserialize_variant(session, data)?),
            "m.login.dummy" => Self::Dummy(deserialize_variant(session, data)?),
            "m.registration_token" => Self::RegistrationToken(deserialize_variant(session, data)?),
            "m.login.terms" => Self::Terms(deserialize_variant(session, data)?),
            "m.oauth" | "org.matrix.cross_signing_reset" => {
                Self::OAuth(deserialize_variant(session, data)?)
            }
            _ => Self::_Custom(CustomAuthData {
                auth_type: auth_type.into(),
                session,
                extra: data,
            }),
        })
    }

    /// Creates a new `AuthData::FallbackAcknowledgement` with the given session key.
    pub fn fallback_acknowledgement(session: String) -> Self {
        Self::FallbackAcknowledgement(FallbackAcknowledgement::new(session))
    }

    /// Returns the value of the `type` field, if it exists.
    pub fn auth_type(&self) -> Option<AuthType> {
        match self {
            Self::Password(_) => Some(AuthType::Password),
            Self::ReCaptcha(_) => Some(AuthType::ReCaptcha),
            Self::EmailIdentity(_) => Some(AuthType::EmailIdentity),
            Self::Msisdn(_) => Some(AuthType::Msisdn),
            Self::Dummy(_) => Some(AuthType::Dummy),
            Self::RegistrationToken(_) => Some(AuthType::RegistrationToken),
            Self::FallbackAcknowledgement(_) => None,
            Self::Terms(_) => Some(AuthType::Terms),
            Self::OAuth(_) => Some(AuthType::OAuth),
            Self::_Custom(c) => Some(AuthType::_Custom(PrivOwnedStr(c.auth_type.as_str().into()))),
        }
    }

    /// Returns the value of the `session` field, if it exists.
    pub fn session(&self) -> Option<&str> {
        match self {
            Self::Password(x) => x.session.as_deref(),
            Self::ReCaptcha(x) => x.session.as_deref(),
            Self::EmailIdentity(x) => x.session.as_deref(),
            Self::Msisdn(x) => x.session.as_deref(),
            Self::Dummy(x) => x.session.as_deref(),
            Self::RegistrationToken(x) => x.session.as_deref(),
            Self::FallbackAcknowledgement(x) => Some(&x.session),
            Self::Terms(x) => x.session.as_deref(),
            Self::OAuth(x) => x.session.as_deref(),
            Self::_Custom(x) => x.session.as_deref(),
        }
    }

    /// Returns the associated data.
    ///
    /// The returned JSON object won't contain the `type` and `session` fields, use
    /// [`.auth_type()`][Self::auth_type] / [`.session()`](Self::session) to access those.
    ///
    /// Prefer to use the public variants of `AuthData` where possible; this method is meant to be
    /// used for custom auth types only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        fn serialize<T: Serialize>(obj: T) -> JsonObject {
            match serde_json::to_value(obj).expect("auth data serialization to succeed") {
                JsonValue::Object(obj) => obj,
                _ => panic!("all auth data variants must serialize to objects"),
            }
        }

        match self {
            Self::Password(x) => Cow::Owned(serialize(Password {
                identifier: x.identifier.clone(),
                password: x.password.clone(),
                session: None,
            })),
            Self::ReCaptcha(x) => Cow::Owned(serialize(ReCaptcha {
                response: x.response.clone(),
                session: None,
            })),
            Self::EmailIdentity(x) => Cow::Owned(serialize(EmailIdentity {
                thirdparty_id_creds: x.thirdparty_id_creds.clone(),
                session: None,
            })),
            Self::Msisdn(x) => Cow::Owned(serialize(Msisdn {
                thirdparty_id_creds: x.thirdparty_id_creds.clone(),
                session: None,
            })),
            Self::RegistrationToken(x) => Cow::Owned(serialize(RegistrationToken {
                token: x.token.clone(),
                session: None,
            })),
            // These types have no associated data.
            Self::Dummy(_) | Self::FallbackAcknowledgement(_) | Self::Terms(_) | Self::OAuth(_) => {
                Cow::Owned(JsonObject::default())
            }
            Self::_Custom(c) => Cow::Borrowed(&c.extra),
        }
    }
}

impl fmt::Debug for AuthData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print `Password { .. }` instead of `Password(Password { .. })`
        match self {
            Self::Password(inner) => inner.fmt(f),
            Self::ReCaptcha(inner) => inner.fmt(f),
            Self::EmailIdentity(inner) => inner.fmt(f),
            Self::Msisdn(inner) => inner.fmt(f),
            Self::Dummy(inner) => inner.fmt(f),
            Self::RegistrationToken(inner) => inner.fmt(f),
            Self::FallbackAcknowledgement(inner) => inner.fmt(f),
            Self::Terms(inner) => inner.fmt(f),
            Self::OAuth(inner) => inner.fmt(f),
            Self::_Custom(inner) => inner.fmt(f),
        }
    }
}

/// Data for password-based UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#password-based
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.password")]
pub struct Password {
    /// One of the user's identifiers.
    pub identifier: UserIdentifier,

    /// The plaintext password.
    pub password: String,

    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl Password {
    /// Creates a new `Password` with the given identifier and password.
    pub fn new(identifier: UserIdentifier, password: String) -> Self {
        Self {
            identifier,
            password,
            session: None,
        }
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            identifier,
            password: _,
            session,
        } = self;
        f.debug_struct("Password")
            .field("identifier", identifier)
            .field("session", session)
            .finish_non_exhaustive()
    }
}

/// Data for ReCaptcha UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#google-recaptcha
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.recaptcha")]
pub struct ReCaptcha {
    /// The captcha response.
    pub response: String,

    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl ReCaptcha {
    /// Creates a new `ReCaptcha` with the given response string.
    pub fn new(response: String) -> Self {
        Self {
            response,
            session: None,
        }
    }
}

impl fmt::Debug for ReCaptcha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            response: _,
            session,
        } = self;
        f.debug_struct("ReCaptcha")
            .field("session", session)
            .finish_non_exhaustive()
    }
}

/// Data for Email-based UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#email-based-identity--homeserver
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.email.identity")]
pub struct EmailIdentity {
    /// Thirdparty identifier credentials.
    #[serde(rename = "threepid_creds")]
    pub thirdparty_id_creds: ThirdpartyIdCredentials,

    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

/// Data for phone number-based UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#phone-numbermsisdn-based-identity--homeserver
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.msisdn")]
pub struct Msisdn {
    /// Thirdparty identifier credentials.
    #[serde(rename = "threepid_creds")]
    pub thirdparty_id_creds: ThirdpartyIdCredentials,

    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

/// Data for dummy UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#dummy-auth
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.dummy")]
pub struct Dummy {
    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl Dummy {
    /// Creates an empty `Dummy`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Data for registration token-based UIAA flow.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#token-authenticated-registration
#[derive(ToSchema, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.registration_token")]
pub struct RegistrationToken {
    /// The registration token.
    pub token: String,

    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl RegistrationToken {
    /// Creates a new `RegistrationToken` with the given token.
    pub fn new(token: String) -> Self {
        Self {
            token,
            session: None,
        }
    }
}

impl fmt::Debug for RegistrationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { token: _, session } = self;
        f.debug_struct("RegistrationToken")
            .field("session", session)
            .finish_non_exhaustive()
    }
}

/// Data for UIAA fallback acknowledgement.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#fallback
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
pub struct FallbackAcknowledgement {
    /// The value of the session key given by the homeserver.
    pub session: String,
}

impl FallbackAcknowledgement {
    /// Creates a new `FallbackAcknowledgement` with the given session key.
    pub fn new(session: String) -> Self {
        Self { session }
    }
}

/// Data for terms of service flow.
///
/// This type is only valid during account registration.
///
/// See [the spec] for how to use this.
///
/// [the spec]: https://spec.matrix.org/latest/client-server-api/#terms-of-service-at-registration
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.login.terms")]
pub struct Terms {
    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl Terms {
    /// Creates an empty `Terms`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Data for an [OAuth 2.0-based] UIAA flow.
///
/// [OAuth 2.0-based]: https://spec.matrix.org/latest/client-server-api/#oauth-authentication
#[derive(ToSchema, Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.oauth")]
pub struct OAuth {
    /// The value of the session key given by the homeserver, if any.
    pub session: Option<String>,
}

impl OAuth {
    /// Construct an empty `OAuth`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Data for an unsupported authentication type.
#[doc(hidden)]
#[derive(Clone, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CustomAuthData {
    /// The type of authentication.
    #[serde(rename = "type")]
    auth_type: String,

    /// The value of the session key given by the homeserver, if any.
    session: Option<String>,

    /// Extra data.
    #[serde(flatten)]
    extra: JsonObject,
}

impl fmt::Debug for CustomAuthData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            auth_type,
            session,
            extra: _,
        } = self;
        f.debug_struct("CustomAuthData")
            .field("auth_type", auth_type)
            .field("session", session)
            .finish_non_exhaustive()
    }
}

/// Identification information for the user.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum UserIdentifier {
    /// Either a fully qualified Matrix user ID, or just the localpart.
    Matrix(MatrixUserIdentifier),

    /// An email address.
    Email(EmailUserIdentifier),

    /// A phone number in the MSISDN format.
    Msisdn(MsisdnUserIdentifier),

    /// A phone number as a separate country code and phone number.
    ///
    /// The homeserver will be responsible for canonicalizing this to the MSISDN format.
    PhoneNumber(PhoneNumberUserIdentifier),

    /// Unsupported `m.id.thirdparty`.
    #[doc(hidden)]
    _CustomThirdParty(CustomThirdPartyUserIdentifier),

    /// Unsupported user identifier.
    #[doc(hidden)]
    _Custom(CustomUserIdentifier),
}

impl UserIdentifier {
    /// Returns the type of this `UserIdentifier`.
    pub fn identifier_type(&self) -> &str {
        match self {
            Self::Matrix(_) => "m.id.user",
            Self::Email(_) | Self::Msisdn(_) | Self::_CustomThirdParty(_) => "m.id.thirdparty",
            Self::PhoneNumber(_) => "m.id.phone",
            Self::_Custom(c) => &c.identifier_type,
        }
    }

    /// Creates a new `UserIdentifier` from the given third party identifier.
    pub fn third_party_id(medium: Medium, address: String) -> Self {
        match medium {
            Medium::Email => Self::Email(EmailUserIdentifier::new(address)),
            Medium::Msisdn => Self::Msisdn(MsisdnUserIdentifier::new(address)),
            _ => Self::_CustomThirdParty(CustomThirdPartyUserIdentifier { medium, address }),
        }
    }

    /// Get this `UserIdentifier` as a third party identifier if it is one.
    pub fn as_third_party_id(&self) -> Option<(&Medium, &str)> {
        match self {
            Self::Email(id) => Some((&Medium::Email, &id.address)),
            Self::Msisdn(id) => Some((&Medium::Msisdn, &id.number)),
            Self::_CustomThirdParty(id) => Some((&id.medium, &id.address)),
            _ => None,
        }
    }

    /// Get the data of this `UserIdentifier` if it is a custom identifier.
    pub fn custom_identifier_data(&self) -> Option<&JsonObject> {
        match self {
            Self::_Custom(c) => Some(&c.data),
            _ => None,
        }
    }
}

impl From<OwnedUserId> for UserIdentifier {
    fn from(id: OwnedUserId) -> Self {
        Self::Matrix(id.into())
    }
}

impl From<&OwnedUserId> for UserIdentifier {
    fn from(id: &OwnedUserId) -> Self {
        Self::Matrix(id.into())
    }
}

/// Data for a Matrix user identifier.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.id.user")]
pub struct MatrixUserIdentifier {
    /// Either a fully qualified Matrix user ID, or just the localpart.
    pub user: String,
}

impl MatrixUserIdentifier {
    /// Creates a new `MatrixUserIdentifier` with the given user.
    pub fn new(user: String) -> Self {
        Self { user }
    }
}

impl From<OwnedUserId> for MatrixUserIdentifier {
    fn from(id: OwnedUserId) -> Self {
        Self { user: id.into() }
    }
}

impl From<&OwnedUserId> for MatrixUserIdentifier {
    fn from(id: &OwnedUserId) -> Self {
        Self {
            user: id.as_str().to_owned(),
        }
    }
}

impl From<MatrixUserIdentifier> for UserIdentifier {
    fn from(id: MatrixUserIdentifier) -> Self {
        Self::Matrix(id)
    }
}

/// Data for an email user identifier.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq)]
pub struct EmailUserIdentifier {
    /// The email address.
    pub address: String,
}

impl EmailUserIdentifier {
    /// Creates a new `EmailUserIdentifier` with the given address.
    pub fn new(address: String) -> Self {
        Self { address }
    }
}

impl From<EmailUserIdentifier> for UserIdentifier {
    fn from(id: EmailUserIdentifier) -> Self {
        Self::Email(id)
    }
}

/// Data for an MSISDN user identifier.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq)]
pub struct MsisdnUserIdentifier {
    /// The phone number according to the [E.164] numbering plan.
    ///
    /// [E.164]: https://www.itu.int/rec/T-REC-E.164-201011-I/en
    pub number: String,
}

impl MsisdnUserIdentifier {
    /// Creates a new `MsisdnUserIdentifier` with the given number.
    pub fn new(number: String) -> Self {
        Self { number }
    }
}

impl From<MsisdnUserIdentifier> for UserIdentifier {
    fn from(id: MsisdnUserIdentifier) -> Self {
        Self::Msisdn(id)
    }
}

/// Data for a phone number user identifier.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.id.phone")]
pub struct PhoneNumberUserIdentifier {
    /// The country that the phone number is from.
    ///
    /// This is a two-letter uppercase [ISO-3166-1 alpha-2] country code.
    ///
    /// [ISO-3166-1 alpha-2]: https://www.iso.org/iso-3166-country-codes.html
    pub country: String,

    /// The phone number.
    pub phone: String,
}

impl PhoneNumberUserIdentifier {
    /// Creates a new `PhoneNumberUserIdentifier` with the given country and phone.
    pub fn new(country: String, phone: String) -> Self {
        Self { country, phone }
    }
}

impl From<PhoneNumberUserIdentifier> for UserIdentifier {
    fn from(id: PhoneNumberUserIdentifier) -> Self {
        Self::PhoneNumber(id)
    }
}

/// Data for an unsupported third-party ID.
#[doc(hidden)]
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename = "m.id.thirdparty")]
pub struct CustomThirdPartyUserIdentifier {
    /// The kind of the third-party ID.
    medium: Medium,

    /// The third-party ID.
    address: String,
}

/// Data for an unsupported user identifier.
#[doc(hidden)]
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomUserIdentifier {
    /// The type of the identifier.
    #[serde(rename = "type")]
    identifier_type: String,

    /// Extra data.
    #[serde(flatten)]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    data: JsonObject,
}

/// Credentials for third-party authentication (e.g. email / phone number).
#[derive(ToSchema, Clone, Deserialize, Serialize)]
pub struct ThirdpartyIdCredentials {
    /// Identity server (or homeserver) session ID.
    pub sid: OwnedSessionId,

    /// Identity server (or homeserver) client secret.
    pub client_secret: OwnedClientSecret,

    /// Identity server URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_server: Option<String>,

    /// Identity server access token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_access_token: Option<String>,
}

impl ThirdpartyIdCredentials {
    /// Creates a new `ThirdpartyIdCredentials` with the given session ID and client secret.
    pub fn new(sid: OwnedSessionId, client_secret: OwnedClientSecret) -> Self {
        Self {
            sid,
            client_secret,
            id_server: None,
            id_access_token: None,
        }
    }
}

impl fmt::Debug for ThirdpartyIdCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            sid,
            client_secret: _,
            id_server,
            id_access_token,
        } = self;
        f.debug_struct("ThirdpartyIdCredentials")
            .field("sid", sid)
            .field("id_server", id_server)
            .field("id_access_token", id_access_token)
            .finish_non_exhaustive()
    }
}
