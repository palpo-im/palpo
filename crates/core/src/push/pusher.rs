//! Common types for the [push notifications module][push].
//!
//! [push]: https://spec.matrix.org/latest/client-server-api/#push-notifications
//!
//! ## Understanding the types of this module
//!
//! Push rules are grouped in `RuleSet`s, and are grouped in five kinds (for
//! more details about the different kind of rules, see the `Ruleset`
//! documentation, or the specification). These five kinds are, by order of
//! priority:
//!
//! - override rules
//! - content rules
//! - room rules
//! - sender rules
//! - underride rules

use salvo::oapi::ToSchema;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, de};
use serde_json::value::from_value as from_json_value;

use crate::push::PushFormat;
use crate::serde::{JsonObject, JsonValue, RawJsonValue, from_raw_json_value};

/// Information for a pusher using the Push Gateway API.
#[derive(ToSchema, Serialize, Deserialize, Clone, Debug)]
pub struct HttpPusherData {
    /// The URL to use to send notifications to.
    ///
    /// Required if the pusher's kind is http.
    pub url: String,

    /// The format to use when sending notifications to the Push Gateway.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<PushFormat>,

    /// iOS (+ macOS?) specific default payload that will be sent to apple push
    /// notification service.
    ///
    /// For more information, see [Sygnal docs][sygnal].
    ///
    /// [sygnal]: https://github.com/matrix-org/sygnal/blob/main/docs/applications.md#ios-applications-beware
    // Not specified, issue: https://github.com/matrix-org/matrix-spec/issues/921
    #[serde(default, skip_serializing_if = "JsonValue::is_null")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub default_payload: JsonValue,
}

impl HttpPusherData {
    /// Creates a new `HttpPusherData` with the given URL.
    pub fn new(url: String) -> Self {
        Self {
            url,
            format: None,
            default_payload: JsonValue::default(),
        }
    }
}

/// Which kind a pusher is, and the information for that kind.
#[derive(ToSchema, Clone, Debug)]
#[non_exhaustive]
pub enum PusherKind {
    /// A pusher that sends HTTP pokes.
    Http(HttpPusherData),

    /// A pusher that emails the user with unread notifications.
    Email(EmailPusherData),

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(CustomPusherData),
}
impl PusherKind {
    pub fn try_new(kind: &str, data: JsonValue) -> Result<Self, serde_json::Error> {
        match kind {
            "http" => from_json_value(data).map(Self::Http),
            "email" => Ok(Self::Email(EmailPusherData)),
            _ => from_json_value(data).map(Self::_Custom),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            PusherKind::Http(_) => "http",
            PusherKind::Email(_) => "email",
            PusherKind::_Custom(data) => data.kind.as_str(),
        }
    }
    pub fn json_data(&self) -> Result<JsonValue, serde_json::Error> {
        match self {
            PusherKind::Http(data) => serde_json::to_value(data),
            PusherKind::Email(data) => serde_json::to_value(data),
            PusherKind::_Custom(data) => serde_json::to_value(data),
        }
    }
}

impl Serialize for PusherKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut st = serializer.serialize_struct("PusherAction", 3)?;

        match self {
            PusherKind::Http(data) => {
                st.serialize_field("kind", &"http")?;
                st.serialize_field("data", data)?;
            }
            PusherKind::Email(_) => {
                st.serialize_field("kind", &"email")?;
                st.serialize_field("data", &JsonObject::new())?;
            }
            PusherKind::_Custom(custom) => {
                st.serialize_field("kind", &custom.kind)?;
                st.serialize_field("data", &custom.data)?;
            }
        }

        st.end()
    }
}

#[derive(Debug, Deserialize)]
struct PusherKindDeHelper {
    kind: String,
    data: Box<RawJsonValue>,
}
impl<'de> Deserialize<'de> for PusherKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;
        let PusherKindDeHelper { kind, data } = from_raw_json_value(&json)?;

        match kind.as_ref() {
            "http" => from_raw_json_value(&data).map(Self::Http),
            "email" => Ok(Self::Email(EmailPusherData)),
            _ => from_raw_json_value(&json).map(Self::_Custom),
        }
    }
}

/// Defines a pusher.
///
/// To create an instance of this type.
#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct Pusher {
    /// Identifiers for this pusher.
    #[serde(flatten)]
    pub ids: PusherIds,

    /// The kind of the pusher and the information for that kind.
    #[serde(flatten)]
    pub kind: PusherKind,

    /// A string that will allow the user to identify what application owns this
    /// pusher.
    pub app_display_name: String,

    /// A string that will allow the user to identify what device owns this
    /// pusher.
    pub device_display_name: String,

    /// The preferred language for receiving notifications (e.g. 'en' or
    /// 'en-US')
    pub lang: String,

    /// Determines which set of device specific rules this pusher executes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PusherDeHelper {
    #[serde(flatten)]
    ids: PusherIds,
    app_display_name: String,
    device_display_name: String,
    profile_tag: Option<String>,
    lang: String,
}

impl<'de> Deserialize<'de> for Pusher {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;

        let PusherDeHelper {
            ids,
            app_display_name,
            device_display_name,
            profile_tag,
            lang,
        } = from_raw_json_value(&json)?;
        let kind = from_raw_json_value(&json)?;

        Ok(Self {
            ids,
            kind,
            app_display_name,
            device_display_name,
            profile_tag,
            lang,
        })
    }
}

/// Strings to uniquely identify a `Pusher`.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A unique identifier for the pusher.
pub struct PusherIds {
    /// The maximum allowed length is 512 bytes.
    pub pushkey: String,

    /// A reverse-DNS style identifier for the application.
    ///
    /// The maximum allowed length is 64 bytes.
    pub app_id: String,
}

impl PusherIds {
    /// Creates a new `PusherIds` with the given pushkey and application ID.
    pub fn new(pushkey: String, app_id: String) -> Self {
        Self { pushkey, app_id }
    }
}

/// Information for an email pusher.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug, Default)]
pub struct EmailPusherData;

impl EmailPusherData {
    /// Creates a new empty `EmailPusherData`.
    pub fn new() -> Self {
        Default::default()
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CustomPusherData {
    kind: String,
    data: JsonObject,
}

/// Information for the pusher implementation itself.
///
/// This is the data dictionary passed in at pusher creation minus the `url`
/// key.
///
/// It can be constructed from [`crate::push::HttpPusherData`] with `::from()` /
/// `.into()`.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct PusherData {
    /// The format to use when sending notifications to the Push Gateway.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<PushFormat>,

    /// iOS (+ macOS?) specific default payload that will be sent to apple push
    /// notification service.
    ///
    /// For more information, see [Sygnal docs][sygnal].
    ///
    /// [sygnal]: https://github.com/matrix-org/sygnal/blob/main/docs/applications.md#ios-applications-beware
    // Not specified, issue: https://github.com/matrix-org/matrix-spec/issues/921
    #[serde(default, skip_serializing_if = "JsonValue::is_null")]
    pub default_payload: JsonValue,
}

impl PusherData {
    /// Creates an empty `PusherData`.
    pub fn new() -> Self {
        Default::default()
    }

    /// Returns `true` if all fields are `None`.
    pub fn is_empty(&self) -> bool {
        #[cfg(not(feature = "unstable-unspecified"))]
        {
            self.format.is_none()
        }

        #[cfg(feature = "unstable-unspecified")]
        {
            self.format.is_none() && self.default_payload.is_null()
        }
    }
}

impl From<crate::push::HttpPusherData> for PusherData {
    fn from(data: crate::push::HttpPusherData) -> Self {
        let crate::push::HttpPusherData {
            format,
            default_payload,
            ..
        } = data;

        Self {
            format,
            default_payload,
        }
    }
}
