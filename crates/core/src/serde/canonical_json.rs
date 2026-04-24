//! Canonical JSON types and related functions.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

mod redaction;
mod serializer;
mod value;

pub use self::redaction::{
    JsonType, RedactedBecause, RedactionError, RedactionEvent, redact, redact_content_in_place,
    redact_in_place,
};
pub use self::serializer::Serializer as CanonicalJsonSerializer;
pub use self::value::{CanonicalJsonObject, CanonicalJsonValue};

pub(crate) const CANONICALJSON_MAX_INT: i64 = (2i64.pow(53)) - 1;
pub(crate) const CANONICALJSON_MIN_INT: i64 = -CANONICALJSON_MAX_INT;

/// The set of possible errors when serializing to canonical JSON.
#[derive(Debug)]
#[allow(clippy::exhaustive_enums)]
pub enum CanonicalJsonError {
    /// The integer value is out of canonical JSON range.
    IntegerOutOfRange,

    /// The given type cannot be serialized to canonical JSON.
    InvalidType(String),

    /// The given type cannot be serialized to an object key.
    InvalidObjectKeyType(String),

    /// The same object key was serialized twice.
    DuplicateObjectKey(String),

    /// An error occurred while re-serializing a [`serde_json::value::RawValue`].
    InvalidRawValue(serde_json::Error),

    /// Another error happened.
    Other(String),
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntegerOutOfRange => f.write_str("integer is out of range for canonical JSON"),
            Self::InvalidType(ty) => {
                write!(f, "{ty} cannot be serialized as canonical JSON")
            }
            Self::InvalidObjectKeyType(ty) => {
                write!(
                    f,
                    "{ty} cannot be used as an object key, expected a string type"
                )
            }
            Self::InvalidRawValue(error) => {
                write!(f, "invalid raw value: {error}")
            }
            Self::DuplicateObjectKey(key) => {
                write!(f, "duplicate object key `{key}`")
            }
            Self::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

impl serde::ser::Error for CanonicalJsonError {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self::Other(msg.to_string())
    }
}

impl From<serde_json::Error> for CanonicalJsonError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidRawValue(value)
    }
}

/// Fallible conversion from a `serde_json::Map` to a `CanonicalJsonObject`.
pub fn try_from_json_map(
    json: serde_json::Map<String, JsonValue>,
) -> Result<CanonicalJsonObject, CanonicalJsonError> {
    json.into_iter()
        .map(|(k, v)| Ok((k, v.try_into()?)))
        .collect()
}

/// Fallible conversion from any value that implements `Serialize` to a `CanonicalJsonObject`.
///
/// `value` must serialize to an `serde_json::Value::Object`.
pub fn to_canonical_object<T: serde::Serialize>(
    value: T,
) -> Result<CanonicalJsonObject, CanonicalJsonError> {
    match to_canonical_value(value)? {
        CanonicalJsonValue::Object(obj) => Ok(obj),
        _ => Err(CanonicalJsonError::Other(
            "Value must be an object".to_owned(),
        )),
    }
}

/// Fallible conversion from any value that impl's `Serialize` to a
/// `CanonicalJsonValue`.
///
/// Uses a custom serializer that correctly handles `serde_json::value::RawValue`
/// and rejects floats, duplicate keys, and out-of-range integers.
pub fn to_canonical_value<T: Serialize>(
    value: T,
) -> Result<CanonicalJsonValue, CanonicalJsonError> {
    value.serialize(CanonicalJsonSerializer)
}

pub fn from_canonical_value<T>(value: CanonicalJsonObject) -> Result<T, CanonicalJsonError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value)?).map_err(CanonicalJsonError::from)
}

pub fn validate_canonical_json(json: &CanonicalJsonObject) -> Result<(), CanonicalJsonError> {
    for value in json.values() {
        match value {
            CanonicalJsonValue::Object(obj) => validate_canonical_json(obj)?,
            CanonicalJsonValue::Array(arr) => {
                for item in arr {
                    if let CanonicalJsonValue::Object(obj) = item {
                        validate_canonical_json(obj)?
                    }
                }
            }
            CanonicalJsonValue::Integer(value) => {
                if *value < CANONICALJSON_MIN_INT || *value > CANONICALJSON_MAX_INT {
                    return Err(CanonicalJsonError::IntegerOutOfRange);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use assert_matches2::assert_matches;
    use serde_json::{
        from_str as from_json_str, json, to_string as to_json_string, to_value as to_json_value,
    };

    use super::value::CanonicalJsonValue;
    use super::{
        CanonicalJsonError, redact, redact_in_place, to_canonical_object, to_canonical_value,
        try_from_json_map,
    };
    use crate::room_version_rules::RedactionRules;

    #[test]
    fn serialize_canon() {
        let json: CanonicalJsonValue = json!({
            "a": [1, 2, 3],
            "other": { "stuff": "hello" },
            "string": "Thing"
        })
        .try_into()
        .unwrap();

        let ser = to_json_string(&json).unwrap();
        let back = from_json_str::<CanonicalJsonValue>(&ser).unwrap();

        assert_eq!(json, back);
    }

    #[test]
    fn check_canonical_sorts_keys() {
        let json: CanonicalJsonValue = json!({
            "auth": {
                "success": true,
                "mxid": "@john.doe:example.com",
                "profile": {
                    "display_name": "John Doe",
                    "three_pids": [
                        {
                            "medium": "email",
                            "address": "john.doe@example.org"
                        },
                        {
                            "medium": "msisdn",
                            "address": "123456789"
                        }
                    ]
                }
            }
        })
        .try_into()
        .unwrap();

        assert_eq!(
            to_json_string(&json).unwrap(),
            r#"{"auth":{"mxid":"@john.doe:example.com","profile":{"display_name":"John Doe","three_pids":[{"address":"john.doe@example.org","medium":"email"},{"address":"123456789","medium":"msisdn"}]},"success":true}}"#
        );
    }

    #[test]
    fn serialize_map_to_canonical() {
        let mut expected = BTreeMap::new();
        expected.insert("foo".into(), CanonicalJsonValue::String("string".into()));
        expected.insert(
            "bar".into(),
            CanonicalJsonValue::Array(vec![
                CanonicalJsonValue::Integer(0),
                CanonicalJsonValue::Integer(1),
                CanonicalJsonValue::Integer(2),
            ]),
        );

        let mut map = serde_json::Map::new();
        map.insert("foo".into(), json!("string"));
        map.insert("bar".into(), json!(vec![0, 1, 2,]));

        assert_eq!(try_from_json_map(map).unwrap(), expected);
    }

    #[test]
    fn to_canonical() {
        #[derive(Debug, serde::Serialize)]
        struct Thing {
            foo: String,
            bar: Vec<u8>,
        }
        let t = Thing {
            foo: "string".into(),
            bar: vec![0, 1, 2],
        };

        let mut expected = BTreeMap::new();
        expected.insert("foo".into(), CanonicalJsonValue::String("string".into()));
        expected.insert(
            "bar".into(),
            CanonicalJsonValue::Array(vec![
                CanonicalJsonValue::Integer(0),
                CanonicalJsonValue::Integer(1),
                CanonicalJsonValue::Integer(2),
            ]),
        );

        assert_eq!(
            to_canonical_value(t).unwrap(),
            CanonicalJsonValue::Object(expected)
        );
    }

    #[test]
    fn redact_allowed_keys_some() {
        let original_event = json!({
            "content": {
                "ban": 50,
                "events": {
                    "m.room.avatar": 50,
                    "m.room.canonical_alias": 50,
                    "m.room.history_visibility": 100,
                    "m.room.name": 50,
                    "m.room.power_levels": 100
                },
                "events_default": 0,
                "invite": 0,
                "kick": 50,
                "redact": 50,
                "state_default": 50,
                "users": {
                    "@example:localhost": 100
                },
                "users_default": 0
            },
            "event_id": "$15139375512JaHAW:localhost",
            "origin_server_ts": 45,
            "sender": "@example:localhost",
            "room_id": "!room:localhost",
            "state_key": "",
            "type": "m.room.power_levels",
            "unsigned": {
                "age": 45
            }
        });

        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(mut object))
        );

        redact_in_place(&mut object, &RedactionRules::V1, None).unwrap();

        let redacted_event = to_json_value(&object).unwrap();

        assert_eq!(
            redacted_event,
            json!({
                "content": {
                    "ban": 50,
                    "events": {
                        "m.room.avatar": 50,
                        "m.room.canonical_alias": 50,
                        "m.room.history_visibility": 100,
                        "m.room.name": 50,
                        "m.room.power_levels": 100
                    },
                    "events_default": 0,
                    "kick": 50,
                    "redact": 50,
                    "state_default": 50,
                    "users": {
                        "@example:localhost": 100
                    },
                    "users_default": 0
                },
                "event_id": "$15139375512JaHAW:localhost",
                "origin_server_ts": 45,
                "sender": "@example:localhost",
                "room_id": "!room:localhost",
                "state_key": "",
                "type": "m.room.power_levels",
            })
        );
    }

    #[test]
    fn redact_allowed_keys_none() {
        let original_event = json!({
            "content": {
                "aliases": ["#somewhere:localhost"]
            },
            "event_id": "$152037280074GZeOm:localhost",
            "origin_server_ts": 1,
            "sender": "@example:localhost",
            "state_key": "room.com",
            "room_id": "!room:room.com",
            "type": "m.room.aliases",
            "unsigned": {
                "age": 1
            }
        });

        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(mut object))
        );

        redact_in_place(&mut object, &RedactionRules::V9, None).unwrap();

        let redacted_event = to_json_value(&object).unwrap();

        assert_eq!(
            redacted_event,
            json!({
                "content": {},
                "event_id": "$152037280074GZeOm:localhost",
                "origin_server_ts": 1,
                "sender": "@example:localhost",
                "state_key": "room.com",
                "room_id": "!room:room.com",
                "type": "m.room.aliases",
            })
        );
    }

    #[test]
    fn redact_allowed_keys_all() {
        let original_event = json!({
            "content": {
              "m.federate": true,
              "predecessor": {
                "event_id": "$something",
                "room_id": "!oldroom:example.org"
              },
              "room_version": "11",
            },
            "event_id": "$143273582443PhrSn",
            "origin_server_ts": 1_432_735,
            "room_id": "!jEsUZKDJdhlrceRyVU:example.org",
            "sender": "@example:example.org",
            "state_key": "",
            "type": "m.room.create",
            "unsigned": {
              "age": 1234,
            },
        });

        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(mut object))
        );

        redact_in_place(&mut object, &RedactionRules::V11, None).unwrap();

        let redacted_event = to_json_value(&object).unwrap();

        assert_eq!(
            redacted_event,
            json!({
                "content": {
                  "m.federate": true,
                  "predecessor": {
                    "event_id": "$something",
                    "room_id": "!oldroom:example.org"
                  },
                  "room_version": "11",
                },
                "event_id": "$143273582443PhrSn",
                "origin_server_ts": 1_432_735,
                "room_id": "!jEsUZKDJdhlrceRyVU:example.org",
                "sender": "@example:example.org",
                "state_key": "",
                "type": "m.room.create",
            })
        );
    }

    #[test]
    fn redact_power_levels_v11_keeps_invite() {
        let original_event = json!({
            "content": {
                "ban": 50,
                "events": { "m.room.power_levels": 100 },
                "events_default": 0,
                "invite": 0,
                "kick": 50,
                "redact": 50,
                "state_default": 50,
                "users": { "@example:localhost": 100 },
                "users_default": 0,
                "custom": "removed"
            },
            "event_id": "$15139375512JaHAW:localhost",
            "origin_server_ts": 45,
            "sender": "@example:localhost",
            "room_id": "!room:localhost",
            "state_key": "",
            "type": "m.room.power_levels",
            "unsigned": { "age": 45 }
        });
        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(object))
        );

        let redacted = redact(object, &RedactionRules::V11, None).unwrap();

        assert_eq!(
            to_json_value(redacted).unwrap(),
            json!({
                "content": {
                    "ban": 50,
                    "events": { "m.room.power_levels": 100 },
                    "events_default": 0,
                    "invite": 0,
                    "kick": 50,
                    "redact": 50,
                    "state_default": 50,
                    "users": { "@example:localhost": 100 },
                    "users_default": 0
                },
                "event_id": "$15139375512JaHAW:localhost",
                "origin_server_ts": 45,
                "sender": "@example:localhost",
                "room_id": "!room:localhost",
                "state_key": "",
                "type": "m.room.power_levels",
            })
        );
    }

    #[test]
    fn redact_room_aliases_v1_keeps_aliases() {
        let original_event = json!({
            "content": { "aliases": ["#somewhere:localhost"], "custom": true },
            "event_id": "$152037280074GZeOm:localhost",
            "origin_server_ts": 1,
            "sender": "@example:localhost",
            "state_key": "room.com",
            "room_id": "!room:room.com",
            "type": "m.room.aliases",
            "unsigned": { "age": 1 }
        });
        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(object))
        );

        let redacted = redact(object, &RedactionRules::V1, None).unwrap();

        assert_eq!(
            to_json_value(redacted).unwrap(),
            json!({
                "content": { "aliases": ["#somewhere:localhost"] },
                "event_id": "$152037280074GZeOm:localhost",
                "origin_server_ts": 1,
                "sender": "@example:localhost",
                "state_key": "room.com",
                "room_id": "!room:room.com",
                "type": "m.room.aliases",
            })
        );
    }

    #[test]
    fn redact_room_member_v11_keeps_third_party_invite_signed_only() {
        let original_event = json!({
            "content": {
                "membership": "invite",
                "displayname": "Example",
                "third_party_invite": {
                    "display_name": "example",
                    "signed": {
                        "mxid": "@alice:example.org",
                        "signatures": {
                            "magic.forest": {
                                "ed25519:3": "fQpGIW1Snz+pwLZu6sTy2aHy/DYWWTspTJRPyNp0PKkymfIsNffysMl6ObMMFdIJhk6g6pwlIqZ54rxo8SLmAg"
                            }
                        },
                        "token": "abc123"
                    }
                }
            },
            "event_id": "$143273582443PhrSn",
            "origin_server_ts": 1_432_735,
            "room_id": "!jEsUZKDJdhlrceRyVU:example.org",
            "sender": "@example:example.org",
            "state_key": "@alice:example.org",
            "type": "m.room.member",
            "unsigned": { "age": 1234 }
        });
        assert_matches!(
            CanonicalJsonValue::try_from(original_event),
            Ok(CanonicalJsonValue::Object(object))
        );

        let redacted = redact(object, &RedactionRules::V11, None).unwrap();

        assert_eq!(
            to_json_value(redacted).unwrap(),
            json!({
                "content": {
                    "membership": "invite",
                    "third_party_invite": {
                        "signed": {
                            "mxid": "@alice:example.org",
                            "signatures": {
                                "magic.forest": {
                                    "ed25519:3": "fQpGIW1Snz+pwLZu6sTy2aHy/DYWWTspTJRPyNp0PKkymfIsNffysMl6ObMMFdIJhk6g6pwlIqZ54rxo8SLmAg"
                                }
                            },
                            "token": "abc123"
                        }
                    }
                },
                "event_id": "$143273582443PhrSn",
                "origin_server_ts": 1_432_735,
                "room_id": "!jEsUZKDJdhlrceRyVU:example.org",
                "sender": "@example:example.org",
                "state_key": "@alice:example.org",
                "type": "m.room.member",
            })
        );
    }

    #[test]
    fn to_canonical_object_rejects_float() {
        let input = serde_json::json!({ "x": 1.5 });
        assert_matches!(
            to_canonical_object(input),
            Err(CanonicalJsonError::InvalidType(ty))
        );
        assert_eq!(ty, "float", "error payload should be \"float\"");
    }
}
