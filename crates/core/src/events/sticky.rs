//! Types for sticky events defined in [MSC4354].
//!
//! [MSC4354]: https://github.com/matrix-org/matrix-spec-proposals/pull/4354

use std::fmt::Formatter;

use salvo::oapi::ToSchema;
use serde::de::Error;
use serde::{Deserialize, Serialize};

/// Sticky duration in milliseconds.
///
/// Valid values are in the integer range 0 through 3,600,000 (one hour).
#[derive(ToSchema, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct StickyDurationMs(u32);

/// Top-level sticky configuration for an event.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct StickyObject {
    /// The number of milliseconds for which the event should be sticky.
    pub duration_ms: StickyDurationMs,
}

impl StickyDurationMs {
    /// The maximum possible sticky duration in milliseconds (one hour).
    pub const MAX: u32 = 3_600_000;

    /// Creates a sticky duration by clamping the value to the supported range.
    pub fn new_clamped<T: Into<u64>>(value: T) -> Self {
        Self(value.into().min(Self::MAX as u64) as u32)
    }

    /// Returns the duration in milliseconds.
    pub fn get(self) -> u32 {
        self.into()
    }
}

impl From<StickyDurationMs> for u32 {
    fn from(duration: StickyDurationMs) -> Self {
        duration.0
    }
}

impl<'de> Deserialize<'de> for StickyDurationMs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StickyDurationMsVisitor;

        impl serde::de::Visitor<'_> for StickyDurationMsVisitor {
            type Value = StickyDurationMs;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an integer in the range 0-3600000")
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if value < 0 {
                    Err(E::invalid_value(
                        serde::de::Unexpected::Signed(value),
                        &self,
                    ))
                } else {
                    self.visit_u64(value as u64)
                }
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if value > StickyDurationMs::MAX as u64 {
                    Err(E::invalid_value(
                        serde::de::Unexpected::Unsigned(value),
                        &self,
                    ))
                } else {
                    Ok(StickyDurationMs(value as u32))
                }
            }
        }

        deserializer.deserialize_any(StickyDurationMsVisitor)
    }
}

/// Deserialize a sticky duration from a URL query value without relaxing the JSON event format.
pub(crate) fn deserialize_query_duration<'de, D>(
    deserializer: D,
) -> Result<Option<StickyDurationMs>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let value = value.parse::<u64>().map_err(D::Error::custom)?;

    if value > StickyDurationMs::MAX as u64 {
        return Err(D::Error::custom(format_args!(
            "sticky duration must be in the range 0-{}",
            StickyDurationMs::MAX
        )));
    }

    Ok(Some(StickyDurationMs(value as u32)))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use assert_matches2::assert_matches;
    use serde_json::{from_value as from_json_value, json};

    use super::{StickyDurationMs, StickyObject};
    use crate::events::{AnyMessageLikeEvent, MessageLikeEvent};
    use crate::serde::CanBeEmpty;

    #[test]
    fn sticky_duration_clamps_constructor_but_rejects_invalid_json() {
        assert_eq!(StickyDurationMs::new_clamped(42_u32).get(), 42);
        assert_eq!(
            StickyDurationMs::new_clamped(StickyDurationMs::MAX as u64 + 1).get(),
            StickyDurationMs::MAX
        );
        assert_eq!(
            StickyDurationMs::new_clamped(u64::MAX).get(),
            StickyDurationMs::MAX
        );

        assert!(serde_json::from_str::<StickyDurationMs>("-1").is_err());
        assert!(serde_json::from_str::<StickyDurationMs>("3600001").is_err());
        assert!(serde_json::from_str::<StickyDurationMs>("3000.0").is_err());
    }

    #[test]
    fn sticky_object_round_trips() {
        let sticky = StickyObject {
            duration_ms: StickyDurationMs::new_clamped(78_000_u32),
        };
        let json = serde_json::to_string(&sticky).unwrap();
        assert_eq!(json, r#"{"duration_ms":78000}"#);

        let sticky: StickyObject = serde_json::from_str(&json).unwrap();
        assert_eq!(sticky.duration_ms.get(), 78_000);
    }

    #[test]
    fn message_event_deserializes_sticky_metadata_and_ttl() {
        let event = from_json_value::<AnyMessageLikeEvent>(json!({
            "content": {
                "body": "Hello, but sticky",
                "msgtype": "m.text"
            },
            "event_id": "$event:example.org",
            "origin_server_ts": 1,
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "type": "m.room.message",
            "msc4354_sticky": { "duration_ms": 3_600_000 },
            "unsigned": { "msc4354_sticky_duration_ttl_ms": 42_000 }
        }))
        .unwrap();

        assert_matches!(
            event,
            AnyMessageLikeEvent::RoomMessage(MessageLikeEvent::Original(event))
        );
        assert_eq!(
            event.sticky.map(|sticky| sticky.duration_ms.get()),
            Some(3_600_000)
        );
        assert_eq!(
            event.unsigned.sticky_duration_ttl_ms,
            Some(Duration::from_millis(42_000))
        );
        assert!(!event.unsigned.is_empty());
    }

    #[test]
    fn invalid_sticky_metadata_does_not_reject_the_event() {
        let event = from_json_value::<AnyMessageLikeEvent>(json!({
            "content": {
                "body": "Hello",
                "msgtype": "m.text"
            },
            "event_id": "$event:example.org",
            "origin_server_ts": 1,
            "room_id": "!room:example.org",
            "sender": "@alice:example.org",
            "type": "m.room.message",
            "msc4354_sticky": { "duration_ms": 3_600_001 }
        }))
        .unwrap();

        assert_matches!(
            event,
            AnyMessageLikeEvent::RoomMessage(MessageLikeEvent::Original(event))
        );
        assert!(event.sticky.is_none());
    }
}
