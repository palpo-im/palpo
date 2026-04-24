//! Custom Serialize / Deserialize implementations for the authentication data types.

use std::borrow::Cow;

use serde::de::Unexpected;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::value::RawValue as RawJsonValue;

use super::{
    AuthData, CustomThirdPartyUserIdentifier, EmailUserIdentifier, MsisdnUserIdentifier,
    UserIdentifier,
};
use crate::serde::from_raw_json_value;
use crate::third_party::Medium;

impl<'de> Deserialize<'de> for AuthData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;

        #[derive(Deserialize)]
        struct ExtractType<'a> {
            #[serde(borrow, rename = "type")]
            auth_type: Option<Cow<'a, str>>,
        }

        let auth_type = serde_json::from_str::<ExtractType<'_>>(json.get())
            .map_err(de::Error::custom)?
            .auth_type;

        match auth_type.as_deref() {
            Some("m.login.password") => from_raw_json_value(&json).map(Self::Password),
            Some("m.login.recaptcha") => from_raw_json_value(&json).map(Self::ReCaptcha),
            Some("m.login.email.identity") => from_raw_json_value(&json).map(Self::EmailIdentity),
            Some("m.login.msisdn") => from_raw_json_value(&json).map(Self::Msisdn),
            Some("m.login.dummy") => from_raw_json_value(&json).map(Self::Dummy),
            Some("m.login.registration_token") => {
                from_raw_json_value(&json).map(Self::RegistrationToken)
            }
            Some("m.login.terms") => from_raw_json_value(&json).map(Self::Terms),
            Some("m.oauth" | "org.matrix.cross_signing_reset") => {
                from_raw_json_value(&json).map(Self::OAuth)
            }
            None => from_raw_json_value(&json).map(Self::FallbackAcknowledgement),
            Some(_) => from_raw_json_value(&json).map(Self::_Custom),
        }
    }
}

impl Serialize for EmailUserIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut id = serializer.serialize_struct("EmailUserIdentifier", 3)?;
        id.serialize_field("type", "m.id.thirdparty")?;
        id.serialize_field("medium", &Medium::Email)?;
        id.serialize_field("address", &self.address)?;
        id.end()
    }
}

impl<'de> Deserialize<'de> for EmailUserIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let CustomThirdPartyUserIdentifier { medium, address } =
            CustomThirdPartyUserIdentifier::deserialize(deserializer)?;

        if medium == Medium::Email {
            Ok(Self { address })
        } else {
            Err(de::Error::invalid_value(
                Unexpected::Str(medium.as_str()),
                &"`email` medium",
            ))
        }
    }
}

impl Serialize for MsisdnUserIdentifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut id = serializer.serialize_struct("MsisdnUserIdentifier", 3)?;
        id.serialize_field("type", "m.id.thirdparty")?;
        id.serialize_field("medium", &Medium::Msisdn)?;
        id.serialize_field("address", &self.number)?;
        id.end()
    }
}

impl<'de> Deserialize<'de> for MsisdnUserIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let CustomThirdPartyUserIdentifier { medium, address } =
            CustomThirdPartyUserIdentifier::deserialize(deserializer)?;

        if medium == Medium::Msisdn {
            Ok(Self { number: address })
        } else {
            Err(de::Error::invalid_value(
                Unexpected::Str(medium.as_str()),
                &"`msisdn` medium",
            ))
        }
    }
}

impl<'de> Deserialize<'de> for UserIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;

        #[derive(Deserialize)]
        struct ExtractType<'a> {
            #[serde(borrow, rename = "type")]
            identifier_type: Cow<'a, str>,
        }

        let id_type = serde_json::from_str::<ExtractType<'_>>(json.get())
            .map_err(de::Error::custom)?
            .identifier_type;

        match id_type.as_ref() {
            "m.id.user" => from_raw_json_value(&json).map(Self::Matrix),
            "m.id.phone" => from_raw_json_value(&json).map(Self::PhoneNumber),
            "m.id.thirdparty" => {
                let CustomThirdPartyUserIdentifier { medium, address } =
                    from_raw_json_value(&json)?;

                match medium {
                    Medium::Email => Ok(Self::Email(EmailUserIdentifier { address })),
                    Medium::Msisdn => Ok(Self::Msisdn(MsisdnUserIdentifier { number: address })),
                    _ => Ok(Self::_CustomThirdParty(CustomThirdPartyUserIdentifier {
                        medium,
                        address,
                    })),
                }
            }
            _ => from_raw_json_value(&json).map(Self::_Custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{
        Value as JsonValue, from_value as from_json_value, json, to_value as to_json_value,
    };

    use crate::client::uiaa::{
        EmailUserIdentifier, MatrixUserIdentifier, MsisdnUserIdentifier, PhoneNumberUserIdentifier,
        UserIdentifier,
    };

    #[test]
    fn serialize_user_identifier_variants() {
        assert_eq!(
            to_json_value(UserIdentifier::Matrix(MatrixUserIdentifier::new(
                "@user:notareal.hs".to_owned()
            )))
            .unwrap(),
            json!({
                "type": "m.id.user",
                "user": "@user:notareal.hs",
            })
        );

        assert_eq!(
            to_json_value(UserIdentifier::PhoneNumber(PhoneNumberUserIdentifier::new(
                "33".to_owned(),
                "0102030405".to_owned(),
            )))
            .unwrap(),
            json!({
                "type": "m.id.phone",
                "country": "33",
                "phone": "0102030405",
            })
        );

        assert_eq!(
            to_json_value(UserIdentifier::Email(EmailUserIdentifier::new(
                "me@myprovider.net".to_owned()
            )))
            .unwrap(),
            json!({
                "type": "m.id.thirdparty",
                "medium": "email",
                "address": "me@myprovider.net",
            })
        );

        assert_eq!(
            to_json_value(UserIdentifier::Msisdn(MsisdnUserIdentifier::new(
                "330102030405".to_owned()
            )))
            .unwrap(),
            json!({
                "type": "m.id.thirdparty",
                "medium": "msisdn",
                "address": "330102030405",
            })
        );

        assert_eq!(
            to_json_value(UserIdentifier::third_party_id(
                "robot".into(),
                "01001110".to_owned()
            ))
            .unwrap(),
            json!({
                "type": "m.id.thirdparty",
                "medium": "robot",
                "address": "01001110",
            })
        );
    }

    #[test]
    fn deserialize_user_identifier_variants() {
        let json = json!({
            "type": "m.id.user",
            "user": "@user:notareal.hs",
        });
        let UserIdentifier::Matrix(id) = from_json_value::<UserIdentifier>(json).unwrap() else {
            panic!("expected Matrix identifier");
        };
        assert_eq!(id.user, "@user:notareal.hs");

        let json = json!({
            "type": "m.id.phone",
            "country": "33",
            "phone": "0102030405",
        });
        let UserIdentifier::PhoneNumber(id) = from_json_value::<UserIdentifier>(json).unwrap()
        else {
            panic!("expected phone identifier");
        };
        assert_eq!(id.country, "33");
        assert_eq!(id.phone, "0102030405");

        let json = json!({
            "type": "m.id.thirdparty",
            "medium": "email",
            "address": "me@myprovider.net",
        });
        let UserIdentifier::Email(id) = from_json_value::<UserIdentifier>(json).unwrap() else {
            panic!("expected email identifier");
        };
        assert_eq!(id.address, "me@myprovider.net");

        let json = json!({
            "type": "m.id.thirdparty",
            "medium": "msisdn",
            "address": "330102030405",
        });
        let UserIdentifier::Msisdn(id) = from_json_value::<UserIdentifier>(json).unwrap() else {
            panic!("expected msisdn identifier");
        };
        assert_eq!(id.number, "330102030405");

        let json = json!({
            "type": "m.id.thirdparty",
            "medium": "robot",
            "address": "01110010",
        });
        let id = from_json_value::<UserIdentifier>(json).unwrap();
        let (medium, address) = id.as_third_party_id().unwrap();
        assert_eq!(medium.as_str(), "robot");
        assert_eq!(address, "01110010");
    }

    #[test]
    fn custom_user_identifier_roundtrips() {
        let json = json!({
            "type": "local.dev.identifier",
            "foo": "bar",
        });

        let id = from_json_value::<UserIdentifier>(json.clone()).unwrap();

        assert_eq!(id.identifier_type(), "local.dev.identifier");
        assert_eq!(
            id.custom_identifier_data().and_then(|data| data.get("foo")),
            Some(&JsonValue::String("bar".to_owned()))
        );
        assert_eq!(to_json_value(id).unwrap(), json);
    }
}
