//! MatrixRTC transport discovery models.
#![allow(deprecated)] // The compatibility module intentionally handles legacy variants.
use std::borrow::Cow;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::serde::JsonObject;

/// Response body for unstable MSC4143 transport discovery.
#[derive(ToSchema, Serialize, Deserialize, Default)]
pub struct RtcTransportsResBody {
    /// Transports advertised by the homeserver.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub rtc_transports: Vec<RtcTransport>,
}
impl RtcTransportsResBody {
    /// Creates a discovery response.
    pub fn new(rtc_transports: Vec<RtcTransport>) -> Self {
        Self { rtc_transports }
    }
}

/// Response body for `/v1/rtc/transports` (MSC4143, pending spec publication).
#[derive(ToSchema, Serialize, Deserialize, Default)]
pub struct StableRtcTransportsResBody {
    /// Transports advertised by the homeserver.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub transports: Vec<RtcTransport>,
}

/// Information about a specific MatrixRTC transport.
#[allow(deprecated)] // for the derives on the deprecated `LegacyLiveKit` variant
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum RtcTransport {
    /// A LiveKit RTC transport, as defined in the merged state of [MSC4195].
    ///
    /// [MSC4195]: https://github.com/matrix-org/matrix-spec-proposals/pull/4195
    #[cfg(feature = "unstable-msc4195")]
    #[serde(rename = "m.livekit")]
    LiveKit(LiveKitRtcTransport),

    /// A LiveKit RTC transport with the `livekit` type used by implementations of earlier
    /// versions of [MSC4195].
    ///
    /// [MSC4195]: https://github.com/matrix-org/matrix-spec-proposals/pull/4195
    #[cfg(feature = "unstable-msc4195")]
    #[serde(rename = "livekit")]
    #[deprecated = "Use `RtcTransport::LiveKit` instead unless you require backwards compatibility with older versions of MSC4195."]
    LegacyLiveKit(LegacyLiveKitRtcTransport),

    /// A custom RTC transport.
    #[doc(hidden)]
    #[serde(untagged)]
    _Custom(CustomRtcTransport),
}

impl<'de> Deserialize<'de> for RtcTransport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        let mut obj = JsonObject::deserialize(deserializer)?;
        let transport_type = match obj.remove("type") {
            Some(serde_json::Value::String(s)) => s,
            Some(_) => return Err(D::Error::custom("`type` must be a string")),
            None => return Err(D::Error::missing_field("type")),
        };

        Ok(match transport_type.as_str() {
            #[cfg(feature = "unstable-msc4195")]
            "m.livekit" => Self::LiveKit(
                serde_json::from_value(serde_json::Value::Object(obj)).map_err(D::Error::custom)?,
            ),
            #[cfg(feature = "unstable-msc4195")]
            #[allow(deprecated)]
            "livekit" | "livekit_multi_sfu" => Self::LegacyLiveKit(
                serde_json::from_value(serde_json::Value::Object(obj)).map_err(D::Error::custom)?,
            ),
            _ => Self::_Custom(CustomRtcTransport {
                transport_type,
                data: obj,
            }),
        })
    }
}

impl RtcTransport {
    /// A constructor to create a custom RTC transport.
    ///
    /// Prefer to use the public variants of `RtcTransport` where possible; this constructor is
    /// meant to be used for unsupported transport types only and does not allow setting arbitrary
    /// data for supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `transport_type` is known and serialization of `data` to the
    /// corresponding `RtcTransport` variant fails.
    pub fn new(transport_type: impl AsRef<str>, data: JsonObject) -> serde_json::Result<Self> {
        let transport_type = transport_type.as_ref();
        Ok(match transport_type {
            #[cfg(feature = "unstable-msc4195")]
            "m.livekit" => Self::LiveKit(serde_json::from_value(serde_json::Value::Object(data))?),
            #[cfg(feature = "unstable-msc4195")]
            #[allow(deprecated)]
            "livekit" | "livekit_multi_sfu" => {
                Self::LegacyLiveKit(serde_json::from_value(serde_json::Value::Object(data))?)
            }
            _ => Self::_Custom(CustomRtcTransport {
                transport_type: transport_type.to_owned(),
                data,
            }),
        })
    }

    #[cfg(feature = "unstable-msc4195")]
    /// Creates a new `RtcTransport::LiveKit` with the given SFU URL.
    pub fn livekit(url: String) -> Self {
        Self::LiveKit(LiveKitRtcTransport::new(url))
    }

    /// Returns a reference to the transport type of this RTC transport.
    pub fn transport_type(&self) -> &str {
        match self {
            #[cfg(feature = "unstable-msc4195")]
            Self::LiveKit(_) => "m.livekit",
            #[cfg(feature = "unstable-msc4195")]
            #[allow(deprecated)]
            Self::LegacyLiveKit(_) => "livekit",
            Self::_Custom(custom) => &custom.transport_type,
        }
    }

    /// Returns the associated data.
    ///
    /// The returned JSON object won't contain the `type` field, please use
    /// [`.transport_type()`][Self::transport_type] to access that.
    ///
    /// Prefer to use the public variants of `RtcTransport` where possible; this method is meant
    /// to be used for custom transport types only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        #[cfg(feature = "unstable-msc4195")]
        fn serialize<T: Serialize>(object: &T) -> JsonObject {
            use serde_json::Value as JsonValue;

            match serde_json::to_value(object).expect("rtc transport type serialization to succeed")
            {
                JsonValue::Object(object) => object,
                _ => panic!("all rtc transport types must serialize to objects"),
            }
        }

        match self {
            #[cfg(feature = "unstable-msc4195")]
            Self::LiveKit(info) => Cow::Owned(serialize(info)),
            #[cfg(feature = "unstable-msc4195")]
            #[allow(deprecated)]
            Self::LegacyLiveKit(info) => Cow::Owned(serialize(info)),
            Self::_Custom(info) => Cow::Borrowed(&info.data),
        }
    }
}

/// Information about an `m.livekit` RTC transport.
#[cfg(feature = "unstable-msc4195")]
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LiveKitRtcTransport {
    /// The WebSocket URL of the LiveKit SFU.
    pub url: String,

    /// The URL for the LiveKit JWT service.
    #[serde(
        rename = "livekit_service_url",
        skip_serializing_if = "Option::is_none"
    )]
    #[deprecated = "Use `url` instead unless you require backwards compatibility with older versions of MSC4195."]
    pub service_url: Option<String>,
}

#[cfg(feature = "unstable-msc4195")]
impl LiveKitRtcTransport {
    /// Creates a new `LiveKitRtcTransport` with the given SFU URL.
    #[allow(deprecated)]
    pub fn new(url: String) -> Self {
        Self {
            url,
            service_url: None,
        }
    }
}

#[cfg(feature = "unstable-msc4195")]
impl From<LiveKitRtcTransport> for RtcTransport {
    fn from(value: LiveKitRtcTransport) -> Self {
        Self::LiveKit(value)
    }
}

/// Information about a legacy `livekit` RTC transport.
#[cfg(feature = "unstable-msc4195")]
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[deprecated = "Use `LiveKitRtcTransport` instead unless you require backwards compatibility with older versions of MSC4195."]
pub struct LegacyLiveKitRtcTransport {
    /// The WebSocket URL of the LiveKit SFU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The URL for the LiveKit JWT service.
    #[serde(
        rename = "livekit_service_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub service_url: Option<String>,
}

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
impl LegacyLiveKitRtcTransport {
    /// Creates a new `LegacyLiveKitRtcTransport` with the given SFU URL.
    pub fn new(url: String) -> Self {
        Self {
            url: Some(url),
            service_url: None,
        }
    }

    /// Creates a new `LegacyLiveKitRtcTransport` with the given JWT service URL.
    pub fn with_service_url(service_url: String) -> Self {
        Self {
            url: None,
            service_url: Some(service_url),
        }
    }
}

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
impl From<LegacyLiveKitRtcTransport> for RtcTransport {
    fn from(value: LegacyLiveKitRtcTransport) -> Self {
        Self::LegacyLiveKit(value)
    }
}

/// Information about a custom RTC transport.
///
/// This type does not implement `Deserialize` to prevent users from
/// constructing the `_Custom` variant of [`RtcTransport`] for a known `type`.
/// Deserialize through [`RtcTransport`] instead.
#[doc(hidden)]
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CustomRtcTransport {
    /// The type of RTC transport.
    #[serde(rename = "type")]
    transport_type: String,

    /// Remaining RTC transport data.
    #[serde(flatten)]
    data: JsonObject,
}

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
#[deprecated = "Use LegacyLiveKitRtcTransport for older transport implementations."]
pub type LivekitMultiSfuTransport = LegacyLiveKitRtcTransport;

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
impl LiveKitRtcTransport {
    /// Construct a legacy service-only transport. Its type remains `livekit`.
    #[deprecated = "Use LegacyLiveKitRtcTransport::with_service_url explicitly."]
    pub fn with_service_url(service_url: String) -> LegacyLiveKitRtcTransport {
        LegacyLiveKitRtcTransport::with_service_url(service_url)
    }
}

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
impl From<crate::client::discovery::homeserver::LiveKitRtcFocusInfo> for RtcTransport {
    fn from(value: crate::client::discovery::homeserver::LiveKitRtcFocusInfo) -> Self {
        match value.url {
            Some(url) => Self::LiveKit(LiveKitRtcTransport {
                url,
                service_url: value.service_url,
            }),
            None => Self::LegacyLiveKit(LegacyLiveKitRtcTransport {
                url: None,
                service_url: value.service_url,
            }),
        }
    }
}
