use as_variant::as_variant;
use salvo::prelude::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeStruct};

use crate::PrivOwnedStr;
use crate::macros::StringEnum;
use crate::serde::{JsonObject, JsonValue, RawJsonValue, from_raw_json_value};

/// This represents the different actions that should be taken when a rule is
/// matched, and controls how notifications are delivered to the client.
///
/// See [the spec](https://spec.matrix.org/latest/client-server-api/#actions) for details.
#[derive(ToSchema, Clone, Debug)]
pub enum Action {
    /// Causes matching events to generate a notification.
    Notify,

    /// Sets an entry in the 'tweaks' dictionary sent to the push gateway.
    SetTweak(Tweak),

    /// An unknown action.
    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(CustomAction),
}

impl Action {
    /// Creates a new `Action`.
    ///
    /// Prefer to use the public variants of `Action` where possible; this constructor is meant
    /// to be used for unsupported actions only and does not allow setting arbitrary data for
    /// supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the action type is known and deserialization of `data` to the
    /// corresponding variant fails.
    pub fn new(data: CustomActionData) -> serde_json::Result<Self> {
        Ok(match data {
            CustomActionData::String(s) => match s.as_str() {
                "notify" => Self::Notify,
                _ => Self::_Custom(CustomAction(CustomActionData::String(s))),
            },
            CustomActionData::Object(o) => {
                if o.contains_key("set_tweak") {
                    Self::SetTweak(serde_json::from_value(JsonValue::Object(o))?)
                } else {
                    Self::_Custom(CustomAction(CustomActionData::Object(o)))
                }
            }
        })
    }

    /// Whether this action is an `Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes))`.
    pub fn is_highlight(&self) -> bool {
        matches!(
            self,
            Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes))
        )
    }

    /// Whether this action should trigger a notification.
    pub fn should_notify(&self) -> bool {
        matches!(self, Action::Notify)
    }

    /// The sound that should be played with this action, if any.
    pub fn sound(&self) -> Option<&SoundTweakValue> {
        as_variant!(self, Action::SetTweak(Tweak::Sound(sound)) => sound)
    }

    /// Access the data if this is a custom action.
    pub fn custom_data(&self) -> Option<&CustomActionData> {
        as_variant!(self, Self::_Custom).map(|action| &action.0)
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(CustomActionData::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

impl Serialize for Action {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Action::Notify => serializer.serialize_str("notify"),
            Action::SetTweak(kind) => kind.serialize(serializer),
            Action::_Custom(custom) => custom.serialize(serializer),
        }
    }
}

/// A custom action.
#[doc(hidden)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CustomAction(CustomActionData);

/// The data of a custom action.
#[allow(unknown_lints, unnameable_types)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CustomActionData {
    /// A string.
    String(String),

    /// An object.
    Object(JsonObject),
}

/// The `set_tweak` action.
#[derive(ToSchema, Clone, Debug)]
pub enum Tweak {
    /// A sound to be played when this notification arrives.
    Sound(SoundTweakValue),

    /// Whether or not this message should be highlighted in the UI.
    Highlight(HighlightTweakValue),

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(CustomTweak),
}

impl Tweak {
    /// Creates a new `Tweak`.
    ///
    /// Prefer to use the public variants of `Tweak` where possible; this constructor is meant
    /// to be used for unsupported tweaks only and does not allow setting arbitrary data for
    /// supported ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the `set_tweak` is known and deserialization of `value` to the
    /// corresponding variant fails.
    pub fn new(set_tweak: String, value: Option<Box<RawJsonValue>>) -> serde_json::Result<Self> {
        Ok(match set_tweak.as_str() {
            "sound" => Self::Sound(from_raw_json_value(
                &value.ok_or_else(|| de::Error::missing_field("value"))?,
            )?),
            "highlight" => {
                let value = value
                    .map(|value| from_raw_json_value::<bool, _>(&value))
                    .transpose()?;

                let highlight = if value.is_none_or(|value| value) {
                    HighlightTweakValue::Yes
                } else {
                    HighlightTweakValue::No
                };

                Self::Highlight(highlight)
            }
            _ => Self::_Custom(CustomTweak { set_tweak, value }),
        })
    }

    /// Access the `set_tweak` value.
    pub fn set_tweak(&self) -> &str {
        match self {
            Self::Sound(_) => "sound",
            Self::Highlight(_) => "highlight",
            Self::_Custom(CustomTweak { set_tweak, .. }) => set_tweak,
        }
    }

    /// Access the value, if this is a custom tweak.
    pub fn custom_value(&self) -> Option<&RawJsonValue> {
        as_variant!(self, Self::_Custom).and_then(|tweak| tweak.value.as_deref())
    }
}

impl<'de> Deserialize<'de> for Tweak {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let CustomTweak { set_tweak, value } = CustomTweak::deserialize(deserializer)?;
        Self::new(set_tweak, value).map_err(de::Error::custom)
    }
}

impl Serialize for Tweak {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Sound(tweak) => {
                let mut s = serializer.serialize_struct("Tweak", 2)?;
                s.serialize_field("set_tweak", &"sound")?;
                s.serialize_field("value", tweak)?;
                s.end()
            }
            Self::Highlight(tweak) => {
                let is_no_highlight = *tweak == HighlightTweakValue::No;
                let len = if is_no_highlight { 2 } else { 1 };

                let mut s = serializer.serialize_struct("Tweak", len)?;
                s.serialize_field("set_tweak", &"highlight")?;

                if is_no_highlight {
                    s.serialize_field("value", &false)?;
                }

                s.end()
            }
            Self::_Custom(tweak) => tweak.serialize(serializer),
        }
    }
}

impl From<SoundTweakValue> for Tweak {
    fn from(value: SoundTweakValue) -> Self {
        Self::Sound(value)
    }
}

impl From<HighlightTweakValue> for Tweak {
    fn from(value: HighlightTweakValue) -> Self {
        Self::Highlight(value)
    }
}

/// A sound to play when a notification arrives.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SoundTweakValue {
    /// Play the default notification sound.
    Default,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// Whether or not a message should be highlighted in the UI.
///
/// This will normally take the form of presenting the message in a different color and/or
/// style. The UI might also be adjusted to draw particular attention to the room in which the
/// event occurred.
#[derive(ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HighlightTweakValue {
    /// Highlight the message.
    #[default]
    Yes,

    /// Don't highlight the message.
    No,
}

impl From<bool> for HighlightTweakValue {
    fn from(value: bool) -> Self {
        if value { Self::Yes } else { Self::No }
    }
}

/// A custom tweak.
#[doc(hidden)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomTweak {
    /// The kind of the custom tweak.
    set_tweak: String,

    /// The value of the custom tweak.
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Box<RawJsonValue>>,
}

#[cfg(test)]
mod tests {
    use assert_matches2::assert_matches;
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{Action, HighlightTweakValue, SoundTweakValue, Tweak};

    #[test]
    fn serialize_string() {
        assert_eq!(to_json_value(Action::Notify).unwrap(), json!("notify"));
    }

    #[test]
    fn serialize_tweak_sound() {
        assert_eq!(
            to_json_value(Action::SetTweak(Tweak::Sound(SoundTweakValue::Default))).unwrap(),
            json!({ "set_tweak": "sound", "value": "default" })
        );
    }

    #[test]
    fn serialize_tweak_highlight() {
        assert_eq!(
            to_json_value(Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes))).unwrap(),
            json!({ "set_tweak": "highlight" })
        );

        assert_eq!(
            to_json_value(Action::SetTweak(Tweak::Highlight(HighlightTweakValue::No))).unwrap(),
            json!({ "set_tweak": "highlight", "value": false })
        );
    }

    #[test]
    fn deserialize_string() {
        assert_matches!(
            from_json_value::<Action>(json!("notify")),
            Ok(Action::Notify)
        );
    }

    #[test]
    fn deserialize_tweak_sound() {
        let json_data = json!({
            "set_tweak": "sound",
            "value": "default"
        });
        assert_matches!(
            from_json_value::<Action>(json_data),
            Ok(Action::SetTweak(Tweak::Sound(value)))
        );
        assert_eq!(value, SoundTweakValue::Default);

        let json_data = json!({
            "set_tweak": "sound",
            "value": "custom"
        });
        assert_matches!(
            from_json_value::<Action>(json_data),
            Ok(Action::SetTweak(Tweak::Sound(value)))
        );
        assert_eq!(value.as_str(), "custom");
    }

    #[test]
    fn deserialize_tweak_highlight() {
        let json_data = json!({
            "set_tweak": "highlight",
            "value": true
        });
        assert_matches!(
            from_json_value::<Action>(json_data),
            Ok(Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes)))
        );
    }

    #[test]
    fn deserialize_tweak_highlight_with_default_value() {
        assert_matches!(
            from_json_value::<Action>(json!({ "set_tweak": "highlight" })),
            Ok(Action::SetTweak(Tweak::Highlight(HighlightTweakValue::Yes)))
        );
    }
}
