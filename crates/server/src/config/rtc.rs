use serde::de::Error;
use serde::{Deserialize, Deserializer};
use url::Url;

use crate::core::client::rtc::transports::RtcTransport;
use crate::macros::config_example;

/// Authenticated MatrixRTC transport discovery (MSC4143).
#[config_example(filename = "palpo-example.toml", section = "rtc")]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RtcConfig {
    /// Enable `/v1/rtc/transports` and the unstable MSC4143 discovery endpoint.
    /// This advertises configured services; it does not enable LiveKit token issuance.
    #[serde(default)]
    pub enabled: bool,

    /// Ordered RTC transport configuration (at most 32 entries).
    /// Set `type = "m.livekit"` and `url = "wss://livekit.example.com"` for a
    /// current LiveKit SFU, or `type = "livekit"` and `livekit_service_url =
    /// "https://jwt.example.com"` for a legacy external JWT service.
    /// These entries are independent from `well_known.rtc_foci`.
    #[serde(default, deserialize_with = "deserialize_transports")]
    pub transports: Vec<RtcTransport>,
}

fn deserialize_transports<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RtcTransport>, D::Error> {
    let transports = Vec::<RtcTransport>::deserialize(d)?;
    if transports.len() > 32 {
        return Err(D::Error::custom(
            "at most 32 RTC transports may be configured",
        ));
    }
    for transport in &transports {
        if transport.transport_type().is_empty() {
            return Err(D::Error::custom("RTC transport type must not be empty"));
        }
        if !matches!(transport.transport_type(), "m.livekit" | "livekit") {
            continue;
        }
        let data = transport.data();
        let sfu = data.get("url").and_then(serde_json::Value::as_str);
        let service = data
            .get("livekit_service_url")
            .and_then(serde_json::Value::as_str);
        if sfu.is_none() && service.is_none() {
            return Err(D::Error::custom(
                "LiveKit transport requires an SFU or JWT service URL",
            ));
        }
        for (value, schemes) in [(sfu, ["ws", "wss"]), (service, ["http", "https"])] {
            if let Some(value) = value {
                let url = Url::parse(value).map_err(D::Error::custom)?;
                if !schemes.contains(&url.scheme())
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    return Err(D::Error::custom(
                        "invalid RTC service URL or embedded credentials",
                    ));
                }
            }
        }
    }
    Ok(transports)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rtc_discovery_defaults_off_and_validates_configured_services() {
        let empty: RtcConfig = serde_json::from_value(json!({})).unwrap();
        assert!(!empty.enabled);
        assert!(empty.transports.is_empty());
        for transport in [
            json!({"type":"m.livekit"}),
            json!({"type":"m.livekit","livekit_service_url":"https://jwt.example"}),
            json!({"type":"livekit"}),
            json!({"type":"m.livekit","url":"https://sfu.example"}),
            json!({"type":"m.livekit","url":""}),
            json!({"type":"m.livekit","url":"wss://secret:password@sfu.example"}),
            json!({"type":"livekit","livekit_service_url":"file:///secret"}),
        ] {
            assert!(
                serde_json::from_value::<RtcConfig>(json!({"transports":[transport]})).is_err()
            );
        }
        assert!(
            serde_json::from_value::<RtcConfig>(
                json!({"transports":vec![json!({"type":"custom"});33]})
            )
            .is_err()
        );
    }
}
