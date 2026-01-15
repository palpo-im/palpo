/// `GET /_matrix/client/*/profile/{user_id}/avatar_url`
///
/// Get the avatar URL of a user.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3profileuser_idavatar_url
use std::borrow::Cow;

use salvo::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{from_value as from_json_value, to_value as to_json_value};

use crate::OwnedMxcUri;
use crate::serde::{JsonValue, StringEnum};

mod profile_field_serde;

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/profile/:user_id/avatar_url",
//         1.1 => "/_matrix/client/v3/profile/:user_id/avatar_url",
//     }
// };

// /// Request type for the `get_avatar_url` endpoint.

// pub struct Requexst {
//     /// The user whose avatar URL will be retrieved.
//     #[salvo(parameter(parameter_in = Path))]
//     pub user_id: OwnedUserId,
// }

/// Response type for the `get_avatar_url` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct AvatarUrlResBody {
    /// The user's avatar URL, if set.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "crate::serde::empty_string_as_none"
    )]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The [BlurHash](https://blurha.sh) for the avatar pointed to by `avatar_url`.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[serde(
        default,
        rename = "xyz.amorgan.blurhash",
        skip_serializing_if = "Option::is_none"
    )]
    pub blurhash: Option<String>,
}
impl AvatarUrlResBody {
    /// Creates a new `Response` with the given avatar URL.
    pub fn new(avatar_url: Option<OwnedMxcUri>) -> Self {
        Self {
            avatar_url,
            blurhash: None,
        }
    }
}

/// `GET /_matrix/client/*/profile/{user_id}/display_name`
///
/// Get the display name of a user.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3profileuser_iddisplay_name
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: None,
//     history: {
//         1.0 => "/_matrix/client/r0/profile/:user_id/display_name",
//         1.1 => "/_matrix/client/v3/profile/:user_id/display_name",
//     }
// };
/// Response type for the `get_display_name` endpoint.
#[derive(ToSchema, Serialize, Deserialize, Debug)]
pub struct DisplayNameResBody {
    /// The user's display name, if set.
    #[serde(
        default,
        rename = "displayname",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_name: Option<String>,
}

impl DisplayNameResBody {
    /// Creates a new `Response` with the given display name.
    pub fn new(display_name: Option<String>) -> Self {
        Self { display_name }
    }
}

// /// `PUT /_matrix/client/*/profile/{user_id}/avatar_url`
// ///
// /// Set the avatar URL of the user.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3profileuser_idavatar_url
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/profile/:user_id/avatar_url",
//         1.1 => "/_matrix/client/v3/profile/:user_id/avatar_url",
//     }
// };

/// Request type for the `set_avatar_url` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetAvatarUrlReqBody {
    /// The new avatar URL for the user.
    ///
    /// `None` is used to unset the avatar.
    #[serde(default, deserialize_with = "crate::serde::empty_string_as_none")]
    pub avatar_url: Option<OwnedMxcUri>,

    /// The [BlurHash](https://blurha.sh) for the avatar pointed to by `avatar_url`.
    ///
    /// This uses the unstable prefix in
    /// [MSC2448](https://github.com/matrix-org/matrix-spec-proposals/pull/2448).
    #[cfg(feature = "unstable-msc2448")]
    #[serde(
        default,
        rename = "xyz.amorgan.blurhash",
        skip_serializing_if = "Option::is_none"
    )]
    pub blurhash: Option<String>,
}

// /// `PUT /_matrix/client/*/profile/{user_id}/display_name`
// ///
// /// Set the display name of the user.
//
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#put_matrixclientv3profileuser_iddisplay_name
// const METADATA: Metadata = metadata! {
//     method: PUT,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/profile/:user_id/display_name",
//         1.1 => "/_matrix/client/v3/profile/:user_id/display_name",
//     }
// };

/// Request type for the `set_display_name` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetDisplayNameReqBody {
    /// The new display name for the user.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "displayname"
    )]
    pub display_name: Option<String>,
}

/// Trait implemented by types representing a field in a user's profile having a statically-known
/// name.
pub trait StaticProfileField {
    /// The type for the value of the field.
    type Value: Sized + Serialize + DeserializeOwned;

    /// The string representation of this field.
    const NAME: &str;
}

/// The user's avatar URL.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::exhaustive_structs)]
pub struct AvatarUrl;

impl StaticProfileField for AvatarUrl {
    type Value = OwnedMxcUri;
    const NAME: &str = "avatar_url";
}

/// The user's display name.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::exhaustive_structs)]
pub struct DisplayName;

impl StaticProfileField for DisplayName {
    type Value = String;
    const NAME: &str = "displayname";
}

/// The possible fields of a user's profile.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
pub enum ProfileFieldName {
    /// The user's avatar URL.
    AvatarUrl,

    /// The user's display name.
    #[palpo_enum(rename = "displayname")]
    DisplayName,

    /// The user's time zone.
    #[palpo_enum(rename = "m.tz")]
    TimeZone,

    #[doc(hidden)]
    _Custom(crate::PrivOwnedStr),
}

/// The possible values of a field of a user's profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProfileFieldValue {
    /// The user's avatar URL.
    AvatarUrl(OwnedMxcUri),

    /// The user's display name.
    #[serde(rename = "displayname")]
    DisplayName(String),

    /// The user's time zone.
    #[serde(rename = "m.tz")]
    TimeZone(String),

    #[doc(hidden)]
    #[serde(untagged)]
    _Custom(CustomProfileFieldValue),
}

impl ProfileFieldValue {
    /// Construct a new `ProfileFieldValue` with the given field and value.
    ///
    /// Prefer to use the public variants of `ProfileFieldValue` where possible; this constructor is
    /// meant to be used for unsupported fields only and does not allow setting arbitrary data for
    /// supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `field` is known and serialization of `value` to the corresponding
    /// `ProfileFieldValue` variant fails.
    pub fn new(field: &str, value: JsonValue) -> serde_json::Result<Self> {
        Ok(match field {
            "avatar_url" => Self::AvatarUrl(from_json_value(value)?),
            "displayname" => Self::DisplayName(from_json_value(value)?),
            _ => Self::_Custom(CustomProfileFieldValue {
                field: field.to_owned(),
                value,
            }),
        })
    }

    /// The name of the field for this value.
    pub fn field_name(&self) -> ProfileFieldName {
        match self {
            Self::AvatarUrl(_) => ProfileFieldName::AvatarUrl,
            Self::DisplayName(_) => ProfileFieldName::DisplayName,
            Self::TimeZone(_) => ProfileFieldName::TimeZone,
            Self::_Custom(CustomProfileFieldValue { field, .. }) => field.as_str().into(),
        }
    }

    /// Returns the value of the field.
    ///
    /// Prefer to use the public variants of `ProfileFieldValue` where possible; this method is
    /// meant to be used for custom fields only.
    pub fn value(&self) -> Cow<'_, JsonValue> {
        match self {
            Self::AvatarUrl(value) => {
                Cow::Owned(to_json_value(value).expect("value should serialize successfully"))
            }
            Self::DisplayName(value) => {
                Cow::Owned(to_json_value(value).expect("value should serialize successfully"))
            }
            Self::TimeZone(value) => {
                Cow::Owned(to_json_value(value).expect("value should serialize successfully"))
            }
            Self::_Custom(c) => Cow::Borrowed(&c.value),
        }
    }
}

/// A custom value for a user's profile field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct CustomProfileFieldValue {
    /// The name of the field.
    field: String,

    /// The value of the field
    value: JsonValue,
}

// /// Endpoint version history valid only for profile fields that didn't exist before Matrix 1.16.
// const EXTENDED_PROFILE_FIELD_HISTORY: VersionHistory = VersionHistory::new(
//     &[(
//         Some("uk.tcpip.msc4133"),
//         "/_matrix/client/unstable/uk.tcpip.msc4133/profile/{user_id}/{field}",
//     )],
//     &[(
//         StablePathSelector::Version(MatrixVersion::V1_16),
//         "/_matrix/client/v3/profile/{user_id}/{field}",
//     )],
//     None,
//     None,
// );

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::ProfileFieldValue;
    use crate::owned_mxc_uri;

    #[test]
    fn serialize_profile_field_value() {
        // Avatar URL.
        let value = ProfileFieldValue::AvatarUrl(owned_mxc_uri!("mxc://localhost/abcdef"));
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "avatar_url": "mxc://localhost/abcdef" })
        );

        // Display name.
        let value = ProfileFieldValue::DisplayName("Alice".to_owned());
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "displayname": "Alice" })
        );

        // Custom field.
        let value = ProfileFieldValue::new("custom_field", "value".into()).unwrap();
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "custom_field": "value" })
        );
    }

    #[test]
    fn deserialize_any_profile_field_value() {
        // Avatar URL.
        let json = json!({ "avatar_url": "mxc://localhost/abcdef" });
        assert_eq!(
            from_json_value::<ProfileFieldValue>(json).unwrap(),
            ProfileFieldValue::AvatarUrl(owned_mxc_uri!("mxc://localhost/abcdef"))
        );

        // Display name.
        let json = json!({ "displayname": "Alice" });
        assert_eq!(
            from_json_value::<ProfileFieldValue>(json).unwrap(),
            ProfileFieldValue::DisplayName("Alice".to_owned())
        );

        // Custom field.
        let json = json!({ "custom_field": "value" });
        let value = from_json_value::<ProfileFieldValue>(json).unwrap();
        assert_eq!(value.field_name().as_str(), "custom_field");
        assert_eq!(value.value().as_str(), Some("value"));

        // Error if the object is empty.
        let json = json!({});
        from_json_value::<ProfileFieldValue>(json).unwrap_err();
    }
}
