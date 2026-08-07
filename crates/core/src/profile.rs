use std::borrow::Cow;
use std::collections::{BTreeMap, btree_map};
use std::fmt;

use salvo::prelude::*;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer, de};
use serde_json::{from_value as from_json_value, to_value as to_json_value};

use crate::serde::{JsonValue, StringEnum};
use crate::{OwnedMxcUri, PrivOwnedStr};

/// The possible fields of a user's profile.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
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
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
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
            "m.tz" => Self::TimeZone(from_json_value(value)?),
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

/// All profile information currently known for a user.
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct UserProfile(BTreeMap<String, JsonValue>);

impl UserProfile {
    /// Creates an empty profile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value of a profile field.
    pub fn get(&self, field: &str) -> Option<&JsonValue> {
        self.0.get(field)
    }

    /// Iterates over the fields in this profile.
    pub fn iter(&self) -> btree_map::Iter<'_, String, JsonValue> {
        self.0.iter()
    }

    /// Sets a profile field.
    pub fn set(&mut self, field: String, value: JsonValue) {
        self.0.insert(field, value);
    }

    /// Removes a profile field, returning its previous value.
    pub fn remove(&mut self, field: &str) -> Option<JsonValue> {
        self.0.remove(field)
    }

    /// Whether this profile has no fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Applies an incremental profile update.
    ///
    /// Fields in `updated` are set, fields listed in `removed` are dropped and omitted fields are
    /// preserved. A JSON `null` in `updated` is a stored value, not a removal: per
    /// [`PUT /_matrix/client/v3/profile/{userId}/{keyName}`][put-profile-field] servers that accept
    /// `null` store it rather than treating it as a delete.
    ///
    /// [put-profile-field]: https://spec.matrix.org/v1.19/client-server-api/#put_matrixclientv3profileuseridkeyname
    #[cfg(feature = "unstable-msc4262")]
    pub fn merge(&mut self, profile_update: UserProfileUpdate) {
        let UserProfileUpdate { updated, removed } = profile_update;

        for (field, value) in updated {
            self.0.insert(field, value);
        }
        for field in removed {
            self.0.remove(&field);
        }
    }
}

impl FromIterator<(String, JsonValue)> for UserProfile {
    fn from_iter<T: IntoIterator<Item = (String, JsonValue)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl FromIterator<(ProfileFieldName, JsonValue)> for UserProfile {
    fn from_iter<T: IntoIterator<Item = (ProfileFieldName, JsonValue)>>(iter: T) -> Self {
        iter.into_iter()
            .map(|(field, value)| (field.as_str().to_owned(), value))
            .collect()
    }
}

impl FromIterator<ProfileFieldValue> for UserProfile {
    fn from_iter<T: IntoIterator<Item = ProfileFieldValue>>(iter: T) -> Self {
        iter.into_iter()
            .map(|value| (value.field_name(), value.value().into_owned()))
            .collect()
    }
}

impl IntoIterator for UserProfile {
    type Item = (String, JsonValue);
    type IntoIter = btree_map::IntoIter<String, JsonValue>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// An incremental update to the profile information for a user.
///
/// Fields that were newly set or changed are in `updated`, field names that were cleared from the
/// profile are in `removed` and omitted fields are unchanged. A JSON `null` inside `updated` is a
/// stored value, not a removal.
#[cfg(feature = "unstable-msc4262")]
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UserProfileUpdate {
    /// Profile fields that were newly set or changed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[salvo(schema(value_type = Object, additional_properties = true))]
    pub updated: BTreeMap<String, JsonValue>,

    /// Names of profile fields that were cleared from the profile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
}

#[cfg(feature = "unstable-msc4262")]
impl UserProfileUpdate {
    /// Creates an empty profile update.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the new value of a profile field in this update.
    pub fn get(&self, field: &str) -> Option<&JsonValue> {
        self.updated.get(field)
    }

    /// Iterates over the fields that were newly set or changed.
    pub fn iter(&self) -> btree_map::Iter<'_, String, JsonValue> {
        self.updated.iter()
    }

    /// Records a profile field as newly set or changed.
    pub fn set(&mut self, field: String, value: JsonValue) {
        self.updated.insert(field, value);
    }

    /// Records a profile field as cleared from the profile.
    pub fn remove(&mut self, field: String) {
        self.removed.push(field);
    }

    /// Whether this update carries no changes at all.
    pub fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.removed.is_empty()
    }
}

#[cfg(feature = "unstable-msc4262")]
impl FromIterator<(String, JsonValue)> for UserProfileUpdate {
    fn from_iter<T: IntoIterator<Item = (String, JsonValue)>>(iter: T) -> Self {
        Self {
            updated: iter.into_iter().collect(),
            removed: Vec::new(),
        }
    }
}

#[cfg(feature = "unstable-msc4262")]
impl FromIterator<(ProfileFieldName, JsonValue)> for UserProfileUpdate {
    fn from_iter<T: IntoIterator<Item = (ProfileFieldName, JsonValue)>>(iter: T) -> Self {
        iter.into_iter()
            .map(|(field, value)| (field.as_str().to_owned(), value))
            .collect()
    }
}

#[cfg(feature = "unstable-msc4262")]
impl FromIterator<ProfileFieldValue> for UserProfileUpdate {
    fn from_iter<T: IntoIterator<Item = ProfileFieldValue>>(iter: T) -> Self {
        iter.into_iter()
            .map(|value| (value.field_name(), value.value().into_owned()))
            .collect()
    }
}

#[cfg(feature = "unstable-msc4262")]
impl Extend<(String, JsonValue)> for UserProfileUpdate {
    fn extend<T: IntoIterator<Item = (String, JsonValue)>>(&mut self, iter: T) {
        self.updated.extend(iter);
    }
}

/// A custom value for a user's profile field.
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct CustomProfileFieldValue {
    /// The name of the field.
    field: String,

    /// The value of the field.
    value: JsonValue,
}

impl Serialize for CustomProfileFieldValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.field, &self.value)?;
        map.end()
    }
}

/// Helper type to deserialize [`ProfileFieldValue`].
///
/// If the inner value is set, this will try to deserialize a map entry using this key, otherwise
/// this will deserialize the first key-value pair encountered.
pub struct ProfileFieldValueVisitor(Option<ProfileFieldName>);

impl ProfileFieldValueVisitor {
    /// Creates a visitor that will deserialize any profile field value.
    pub fn new(field: Option<ProfileFieldName>) -> Self {
        Self(field)
    }
}

impl<'de> de::Visitor<'de> for ProfileFieldValueVisitor {
    type Value = Option<ProfileFieldValue>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("enum ProfileFieldValue")
    }

    fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
    where
        V: de::MapAccess<'de>,
    {
        let field = if let Some(field) = self.0 {
            let mut found = false;

            while let Some(key) = map.next_key::<ProfileFieldName>()? {
                if key == field {
                    found = true;
                    break;
                }
            }

            if !found {
                return Ok(None);
            }

            field
        } else {
            let Some(field) = map.next_key()? else {
                return Ok(None);
            };

            field
        };

        Ok(Some(match field {
            ProfileFieldName::AvatarUrl => ProfileFieldValue::AvatarUrl(map.next_value()?),
            ProfileFieldName::DisplayName => ProfileFieldValue::DisplayName(map.next_value()?),
            ProfileFieldName::TimeZone => ProfileFieldValue::TimeZone(map.next_value()?),
            ProfileFieldName::_Custom(field) => {
                ProfileFieldValue::_Custom(CustomProfileFieldValue {
                    field: field.0.into(),
                    value: map.next_value()?,
                })
            }
        }))
    }
}

fn deserialize_profile_field_value_option<'de, D>(
    deserializer: D,
) -> Result<Option<ProfileFieldValue>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserializer.deserialize_map(ProfileFieldValueVisitor::new(None))
}

impl<'de> Deserialize<'de> for ProfileFieldValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        deserialize_profile_field_value_option(deserializer)?
            .ok_or_else(|| de::Error::invalid_length(0, &"at least one key-value pair"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::ProfileFieldValue;
    #[cfg(feature = "unstable-msc4262")]
    use super::{UserProfile, UserProfileUpdate};
    use crate::owned_mxc_uri;

    #[test]
    fn serialize_profile_field_value() {
        let value = ProfileFieldValue::AvatarUrl(owned_mxc_uri!("mxc://localhost/abcdef"));
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "avatar_url": "mxc://localhost/abcdef" })
        );

        let value = ProfileFieldValue::DisplayName("Alice".to_owned());
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "displayname": "Alice" })
        );

        let value = ProfileFieldValue::TimeZone("Europe/Paris".to_owned());
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "m.tz": "Europe/Paris" })
        );

        let value = ProfileFieldValue::new("custom_field", "value".into()).unwrap();
        assert_eq!(
            to_json_value(value).unwrap(),
            json!({ "custom_field": "value" })
        );
    }

    #[cfg(feature = "unstable-msc4262")]
    #[test]
    fn merge_profile_update_preserves_omitted_and_drops_removed_fields() {
        let mut profile = UserProfile::from_iter([
            ("displayname".to_owned(), json!("Alice")),
            ("avatar_url".to_owned(), json!("mxc://example.org/avatar")),
        ]);
        let mut update =
            UserProfileUpdate::from_iter([("m.tz".to_owned(), json!("Europe/Berlin"))]);
        update.remove("avatar_url".to_owned());

        profile.merge(update);

        assert_eq!(profile.get("displayname"), Some(&json!("Alice")));
        assert_eq!(profile.get("avatar_url"), None);
        assert_eq!(profile.get("m.tz"), Some(&json!("Europe/Berlin")));
    }

    /// A `null` in `updated` is a stored value, not a delete: only `removed` clears a field.
    #[cfg(feature = "unstable-msc4262")]
    #[test]
    fn merge_profile_update_stores_explicit_null_values() {
        let mut profile =
            UserProfile::from_iter([("org.example.language".to_owned(), json!("en-GB"))]);
        let update =
            UserProfileUpdate::from_iter([("org.example.language".to_owned(), json!(null))]);

        profile.merge(update);

        assert_eq!(profile.get("org.example.language"), Some(&json!(null)));
    }

    #[test]
    fn deserialize_any_profile_field_value() {
        let json = json!({ "avatar_url": "mxc://localhost/abcdef" });
        assert_eq!(
            from_json_value::<ProfileFieldValue>(json).unwrap(),
            ProfileFieldValue::AvatarUrl(owned_mxc_uri!("mxc://localhost/abcdef"))
        );

        let json = json!({ "displayname": "Alice" });
        assert_eq!(
            from_json_value::<ProfileFieldValue>(json).unwrap(),
            ProfileFieldValue::DisplayName("Alice".to_owned())
        );

        let json = json!({ "m.tz": "Europe/Paris" });
        assert_eq!(
            from_json_value::<ProfileFieldValue>(json).unwrap(),
            ProfileFieldValue::TimeZone("Europe/Paris".to_owned())
        );

        let json = json!({ "custom_field": "value" });
        let value = from_json_value::<ProfileFieldValue>(json).unwrap();
        assert_eq!(value.field_name().as_str(), "custom_field");
        assert_eq!(value.value().as_str(), Some("value"));

        let json = json!({});
        from_json_value::<ProfileFieldValue>(json).unwrap_err();
    }
}
