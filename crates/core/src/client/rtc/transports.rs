//! `GET /_matrix/client/*/rtc/transports`
//!
//! Discover the RTC transports advertised by the homeserver.
//! `/v1/` ([MSC])
//!
//! [MSC]: https://github.com/matrix-org/matrix-spec-proposals/pull/4143

use std::borrow::Cow;

use salvo::oapi::ToSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::serde::JsonObject;

// metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/client/unstable/org.matrix.msc4143/rtc/transports",
//     }
// }

// /// Request type for the `transports` endpoint.
// #[request(error = crate::Error)]
// #[derive(Default)]
// pub struct Request {}

// impl Request {
//     /// Creates a new empty `Request`.
//     pub fn new() -> Self {
//         Self {}
//     }
// }

/// Response type for the `transports` endpoint.
#[derive(ToSchema, Serialize, Default)]
pub struct RtcTransportsResBody {
    /// The RTC transports advertised by the homeserver.
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub rtc_transports: Vec<RtcTransport>,
}

impl RtcTransportsResBody {
    /// Creates a `Response` with the given RTC transports.
    pub fn new(rtc_transports: Vec<RtcTransport>) -> Self {
        Self { rtc_transports }
    }
}

/// A MatrixRTC transport.
///
/// This type can hold arbitrary RTC transports. Their data can be accessed with
/// [`transport_type()`](Self::transport_type) and [`data()`](Self::data()).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum RtcTransport {
    /// A LiveKit RTC transport.
    #[cfg(feature = "unstable-msc4195")]
    #[serde(rename = "livekit", alias = "livekit_multi_sfu")]
    LiveKit(LiveKitRtcTransport),

    /// An unsupported transport.
    #[doc(hidden)]
    #[serde(untagged)]
    _Custom(CustomRtcTransport),
}

impl RtcTransport {
    /// A constructor to create a custom RTC transport.
    ///
    /// Prefer to use the public variants of `RtcTransport` where possible; this constructor is
    /// meant to be used for unsupported focus types only and does not allow setting arbitrary
    /// data for supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `transport_type` is known and deserialization of `data` to the
    /// corresponding `RtcTransport` variant fails.
    pub fn new(transport_type: String, data: JsonObject) -> serde_json::Result<Self> {
        fn deserialize_variant<T: DeserializeOwned>(obj: JsonObject) -> serde_json::Result<T> {
            serde_json::from_value(obj.into())
        }

        Ok(match transport_type.as_str() {
            #[cfg(feature = "unstable-msc4195")]
            "livekit" | "livekit_multi_sfu" => Self::LiveKit(deserialize_variant(data)?),
            _ => Self::_Custom(CustomRtcTransport {
                transport_type,
                data,
            }),
        })
    }

    /// Returns a reference to the type of this RTC transport.
    pub fn transport_type(&self) -> &str {
        match self {
            #[cfg(feature = "unstable-msc4195")]
            Self::LiveKit(_) => "livekit",
            Self::_Custom(custom) => &custom.transport_type,
        }
    }

    /// Returns the associated data.
    ///
    /// The returned JSON object won't contain the `type` field, please use
    /// [`transport_type()`][Self::transport_type] to access that.
    ///
    /// Prefer to use the public variants of `RtcTransport` where possible; this method is meant
    /// to be used for custom focus types only.
    pub fn data(&self) -> Cow<'_, JsonObject> {
        fn serialize<T: Serialize>(object: &T) -> JsonObject {
            match serde_json::to_value(object).expect("rtc focus type serialization to succeed") {
                JsonValue::Object(object) => object,
                _ => panic!("rtc transports must serialize to JSON objects"),
            }
        }

        match self {
            #[cfg(feature = "unstable-msc4195")]
            Self::LiveKit(info) => Cow::Owned(serialize(info)),
            Self::_Custom(info) => Cow::Borrowed(&info.data),
        }
    }
}

/// Information about a LiveKit RTC transport.
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg(feature = "unstable-msc4195")]
pub struct LiveKitRtcTransport {
    /// The WebSocket URL of the LiveKit SFU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// The URL of the legacy LiveKit JWT service.
    #[serde(
        rename = "livekit_service_url",
        skip_serializing_if = "Option::is_none"
    )]
    #[deprecated = "Use `url` unless compatibility with older MSC4195 deployments is required."]
    pub service_url: Option<String>,
}

#[cfg(feature = "unstable-msc4195")]
impl LiveKitRtcTransport {
    /// Construct a LiveKit transport with the given SFU WebSocket URL.
    #[allow(deprecated)]
    pub fn new(url: String) -> Self {
        Self {
            url: Some(url),
            service_url: None,
        }
    }

    /// Construct a LiveKit transport with the legacy JWT service URL.
    #[deprecated = "Use `LiveKitRtcTransport::new()` unless compatibility with older MSC4195 deployments is required."]
    #[allow(deprecated)]
    pub fn with_service_url(service_url: String) -> Self {
        Self {
            url: None,
            service_url: Some(service_url),
        }
    }
}

#[cfg(feature = "unstable-msc4195")]
impl From<LiveKitRtcTransport> for RtcTransport {
    fn from(value: LiveKitRtcTransport) -> Self {
        Self::LiveKit(value)
    }
}

/// Deprecated name for [`LiveKitRtcTransport`].
#[cfg(feature = "unstable-msc4195")]
#[deprecated = "Use `LiveKitRtcTransport`."]
pub type LivekitMultiSfuTransport = LiveKitRtcTransport;

#[cfg(feature = "unstable-msc4195")]
#[allow(deprecated)]
impl From<crate::client::discovery::homeserver::LiveKitRtcFocusInfo> for LiveKitRtcTransport {
    fn from(value: crate::client::discovery::homeserver::LiveKitRtcFocusInfo) -> Self {
        let crate::client::discovery::homeserver::LiveKitRtcFocusInfo { url, service_url } = value;
        Self { url, service_url }
    }
}

/// Information about an unsupported RTC transport.
#[doc(hidden)]
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CustomRtcTransport {
    /// The type of RTC transport.
    #[serde(rename = "type")]
    transport_type: String,

    /// Remaining data.
    #[serde(flatten)]
    data: JsonObject,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as JsonValue, from_value, json, to_value};

    #[cfg(feature = "unstable-msc4195")]
    use super::LiveKitRtcTransport;
    use super::RtcTransport;

    #[test]
    fn serialize_roundtrip_custom_rtc_transport() {
        let JsonValue::Object(transport_data) = json!({ "foo": "bar", "baz": true }) else {
            unreachable!();
        };
        let transport =
            RtcTransport::new("local.custom.transport".to_owned(), transport_data.clone()).unwrap();
        let json = json!({ "type": "local.custom.transport", "foo": "bar", "baz": true });

        assert_eq!(transport.transport_type(), "local.custom.transport");
        assert_eq!(*transport.data().as_ref(), transport_data);
        assert_eq!(to_value(&transport).unwrap(), json);
        assert_eq!(from_value::<RtcTransport>(json).unwrap(), transport);
    }

    #[cfg(feature = "unstable-msc4195")]
    #[test]
    fn serialize_roundtrip_livekit_sfu_transport() {
        let transport =
            RtcTransport::from(LiveKitRtcTransport::new("wss://livekit.local/".to_owned()));
        let json = json!({ "type": "livekit", "url": "wss://livekit.local/" });

        assert_eq!(transport.transport_type(), "livekit");
        assert_eq!(to_value(&transport).unwrap(), json);
        assert_eq!(from_value::<RtcTransport>(json).unwrap(), transport);
    }

    #[cfg(feature = "unstable-msc4195")]
    #[test]
    #[allow(deprecated)]
    fn accepts_legacy_livekit_transport_and_normalizes_legacy_type() {
        let legacy = json!({
            "type": "livekit_multi_sfu",
            "livekit_service_url": "https://livekit-jwt.local/"
        });
        let transport: RtcTransport = from_value(legacy).unwrap();
        let RtcTransport::LiveKit(livekit) = &transport else {
            panic!("legacy LiveKit transport should use the typed variant");
        };

        assert_eq!(livekit.url, None);
        assert_eq!(
            livekit.service_url.as_deref(),
            Some("https://livekit-jwt.local/")
        );
        assert_eq!(
            to_value(transport).unwrap(),
            json!({ "type": "livekit", "livekit_service_url": "https://livekit-jwt.local/" })
        );
    }
}
