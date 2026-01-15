use std::clone::Clone;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::mem;

use salvo::oapi::{Components, RefOr, Schema};
use salvo::prelude::*;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::ser::{Serialize, Serializer};
use serde_json::value::to_raw_value as to_raw_json_value;

use crate::serde::{JsonValue, RawJsonValue};

/// A wrapper around `Box<RawValue>`, to be used in place of any type in the
/// Matrix endpoint definition to allow request and response types to contain
/// that said type represented by the generic argument `Ev`.
///
/// Palpo offers the `RawJson` wrapper to enable passing around JSON text that
/// is only partially validated. This is useful when a client receives events
/// that do not follow the spec perfectly or a server needs to generate
/// reference hashes with the original canonical JSON string. All event structs
/// and enums implement `Serialize` / `Deserialize`, `Raw` should be used
/// to pass around events in a lossless way.
///
/// ```no_run
/// # use serde::Deserialize;
/// # use palpo_core::serde::RawJson;
/// # #[derive(Deserialize)]
/// # struct AnyTimelineEvent;
///
/// let json = r#"{ "type": "imagine a full event", "content": {...} }"#;
///
/// let deser = serde_json::from_str::<RawJson<AnyTimelineEvent>>(json)
///     .unwrap() // the first Result from serde_json::from_str, will not fail
///     .deserialize() // deserialize to the inner type
///     .unwrap(); // finally get to the AnyTimelineEvent
/// ```
#[repr(transparent)]
pub struct RawJson<T> {
    inner: Box<RawJsonValue>,
    _ev: PhantomData<T>,
}
impl<T> ToSchema for RawJson<T>
where
    T: ToSchema + 'static,
{
    fn to_schema(components: &mut Components) -> RefOr<Schema> {
        T::to_schema(components)
    }
}

impl<T> RawJson<T> {
    /// Create a `Raw` by serializing the given `T`.
    ///
    /// Shorthand for
    /// `serde_json::value::to_raw_value(val).map(RawJson::from_json)`, but
    /// specialized to `T`.
    ///
    /// # Errors
    ///
    /// Fails if `T`s [`Serialize`] implementation fails.
    pub fn new(val: &T) -> serde_json::Result<Self>
    where
        T: Serialize,
    {
        to_raw_json_value(val).map(Self::from_raw_value)
    }

    /// Create a `Raw` from a boxed `RawValue`.
    pub fn from_value(val: &JsonValue) -> serde_json::Result<Self> {
        to_raw_json_value(val).map(Self::from_raw_value)
    }
    pub fn from_raw_value(inner: Box<RawJsonValue>) -> Self {
        Self {
            inner,
            _ev: PhantomData,
        }
    }

    /// Convert an owned `String` of JSON data to `RawJson<T>`.
    ///
    /// This function is equivalent to `serde_json::from_str::<RawJson<T>>`
    /// except that an allocation and copy is avoided if both of the
    /// following are true:
    ///
    /// * the input has no leading or trailing whitespace, and
    /// * the input has capacity equal to its length.
    pub fn from_string(json: String) -> serde_json::Result<Self> {
        RawJsonValue::from_string(json).map(Self::from_raw_value)
    }

    /// Access the underlying json value.
    pub fn inner(&self) -> &RawJsonValue {
        &self.inner
    }

    pub fn as_str(&self) -> &str {
        self.inner.get()
    }

    /// Convert `self` into the underlying json value.
    pub fn into_inner(self) -> Box<RawJsonValue> {
        self.inner
    }

    /// Try to access a given field inside this `Raw`, assuming it contains an
    /// object.
    ///
    /// Returns `Err(_)` when the contained value is not an object, or the field
    /// exists but is fails to deserialize to the expected type.
    ///
    /// Returns `Ok(None)` when the field doesn't exist or is `null`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # type CustomMatrixEvent = ();
    /// # fn foo() -> serde_json::Result<()> {
    /// # let raw_event: palpo_core::serde::RawJson<()> = todo!();
    /// if raw_event.get_field::<String>("type")?.as_deref() == Some("org.custom.matrix.event") {
    ///     let event = raw_event.deserialize_as::<CustomMatrixEvent>()?;
    ///     // ...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field<'a, U>(&'a self, field_name: &str) -> serde_json::Result<Option<U>>
    where
        U: Deserialize<'a>,
    {
        struct FieldVisitor<'b>(&'b str);

        impl<'b, 'de> Visitor<'de> for FieldVisitor<'b> {
            type Value = bool;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "`{}`", self.0)
            }

            fn visit_str<E>(self, value: &str) -> Result<bool, E>
            where
                E: de::Error,
            {
                Ok(value == self.0)
            }
        }

        struct Field<'b>(&'b str);

        impl<'b, 'de> DeserializeSeed<'de> for Field<'b> {
            type Value = bool;

            fn deserialize<D>(self, deserializer: D) -> Result<bool, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_identifier(FieldVisitor(self.0))
            }
        }

        struct SingleFieldVisitor<'b, T> {
            field_name: &'b str,
            _phantom: PhantomData<T>,
        }

        impl<'b, T> SingleFieldVisitor<'b, T> {
            fn new(field_name: &'b str) -> Self {
                Self {
                    field_name,
                    _phantom: PhantomData,
                }
            }
        }

        impl<'b, 'de, T> Visitor<'de> for SingleFieldVisitor<'b, T>
        where
            T: Deserialize<'de>,
        {
            type Value = Option<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut res = None;
                while let Some(is_right_field) = map.next_key_seed(Field(self.field_name))? {
                    if is_right_field {
                        res = Some(map.next_value()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }

                Ok(res)
            }
        }

        let mut deserializer = serde_json::Deserializer::from_str(self.inner().get());
        deserializer.deserialize_map(SingleFieldVisitor::new(field_name))
    }

    /// Try to deserialize the JSON as the expected type.
    pub fn deserialize<'a>(&'a self) -> serde_json::Result<T>
    where
        T: Deserialize<'a>,
    {
        serde_json::from_str(self.inner.get())
    }

    /// Try to deserialize the JSON as a custom type.
    pub fn deserialize_as<'a, U>(&'a self) -> serde_json::Result<U>
    where
        U: Deserialize<'a>,
    {
        serde_json::from_str(self.inner.get())
    }

    /// Turns `RawJson<T>` into `RawJson<U>` without changing the underlying
    /// JSON.
    ///
    /// This is useful for turning raw specific event types into raw event enum
    /// types.
    pub fn cast<U>(self) -> RawJson<U> {
        RawJson::from_raw_value(self.into_inner())
    }

    /// Turns `&RawJson<T>` into `&RawJson<U>` without changing the underlying
    /// JSON.
    ///
    /// This is useful for turning raw specific event types into raw event enum
    /// types.
    pub fn cast_ref<U>(&self) -> &RawJson<U> {
        unsafe { mem::transmute(self) }
    }
}

impl<T> Clone for RawJson<T> {
    fn clone(&self) -> Self {
        Self::from_raw_value(self.inner.clone())
    }
}

impl<T> Debug for RawJson<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use std::any::type_name;
        f.debug_struct(&format!("RawJson::<{}>", type_name::<T>()))
            .field("json", &self.inner)
            .finish()
    }
}

impl<'de, T> Deserialize<'de> for RawJson<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Box::<RawJsonValue>::deserialize(deserializer).map(Self::from_raw_value)
    }
}

impl<T> Serialize for RawJson<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.inner.serialize(serializer)
    }
}

/// Marker trait for restricting the types [`Raw::deserialize_as`], [`Raw::cast`] and
/// [`Raw::cast_ref`] can be called with.
///
/// Implementing this trait for a type `U` means that it is safe to cast from `U` to `T` because `T`
/// can be deserialized from the same JSON as `U`.
pub trait JsonCastable<T> {}

impl<T> JsonCastable<JsonValue> for T {}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::from_str as from_json_str;

    use crate::serde::{RawJson, RawJsonValue};

    #[test]
    fn get_field() -> serde_json::Result<()> {
        #[derive(Debug, PartialEq, Deserialize)]
        struct A<'a> {
            #[serde(borrow)]
            b: Vec<&'a str>,
        }

        const OBJ: &str = r#"{ "a": { "b": [  "c"] }, "z": 5 }"#;
        let raw: RawJson<()> = from_json_str(OBJ)?;

        assert_eq!(raw.get_field::<u8>("z")?, Some(5));
        assert_eq!(
            raw.get_field::<&RawJsonValue>("a")?.unwrap().get(),
            r#"{ "b": [  "c"] }"#
        );
        assert_eq!(raw.get_field::<A<'_>>("a")?, Some(A { b: vec!["c"] }));

        assert_eq!(raw.get_field::<u8>("b")?, None);
        raw.get_field::<u8>("a").unwrap_err();

        Ok(())
    }
}
