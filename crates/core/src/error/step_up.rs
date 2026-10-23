//! Validated MSC4363 OAuth authentication challenges.
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};

/// A validated space-separated list of OAuth scopes (RFC 6749 section 3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ScopeValues(String);

impl ScopeValues {
    /// Parse a nonempty list of RFC 6749 scope tokens.
    pub fn parse(value: &str) -> Option<Self> {
        value
            .split(' ')
            .all(|token| {
                !token.is_empty()
                    && token
                        .bytes()
                        .all(|b| matches!(b, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
            })
            .then(|| Self(value.to_owned()))
    }

    /// The space-separated wire representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ScopeValues {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| serde::de::Error::custom("invalid OAuth scope list"))
    }
}

/// A validated, ordered, space-separated list of nonempty OIDC ACR values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AcrValues(String);

impl AcrValues {
    /// Parse ACR tokens in preference order.
    pub fn parse(value: &str) -> Option<Self> {
        value
            .split(' ')
            .all(|token| {
                !token.is_empty() && token.chars().all(|c| !c.is_whitespace() && !c.is_control())
            })
            .then(|| Self(value.to_owned()))
    }

    /// The ACR values, in preference order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.split(' ')
    }
}

impl<'de> Deserialize<'de> for AcrValues {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| serde::de::Error::custom("invalid ACR list"))
    }
}

/// Authentication requirements for a verified OAuth identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepUpChallenge {
    /// Acceptable ACR values in preference order; satisfying any one is sufficient.
    #[serde(
        rename = "org.matrix.msc4363.acr_values",
        alias = "acr_values",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub acr_values: Option<AcrValues>,
    /// Maximum authentication age, serialized as integer seconds.
    #[serde(
        rename = "org.matrix.msc4363.max_age",
        alias = "max_age",
        default,
        with = "crate::serde::duration::opt_secs",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_age: Option<Duration>,
    /// Full set of scopes required by the resource.
    #[serde(
        rename = "org.matrix.msc4363.scope",
        alias = "scope",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub scope: Option<ScopeValues>,
}

#[cfg(test)]
mod tests {
    use salvo::test::ResponseExt;
    use salvo::writing::Scribe;
    use serde_json::{from_value, json, to_value};

    use super::*;
    use crate::error::{ErrorKind, MatrixError};

    #[tokio::test]
    async fn step_up_wire_format_aliases_and_http_status() {
        for code in [
            "M_INSUFFICIENT_USER_AUTHENTICATION",
            "org.matrix.msc4363.M_INSUFFICIENT_USER_AUTHENTICATION",
        ] {
            let error: ErrorKind = from_value(json!({"errcode":code, "org.matrix.msc4363.acr_values":"mfa pwd", "org.matrix.msc4363.max_age":300, "org.matrix.msc4363.scope":"openid urn:matrix:client:api:*"})).unwrap();
            let encoded = to_value(&error).unwrap();
            assert_eq!(encoded["org.matrix.msc4363.max_age"], 300);
            assert_eq!(encoded["org.matrix.msc4363.acr_values"], "mfa pwd");
            assert_eq!(
                encoded["org.matrix.msc4363.scope"],
                "openid urn:matrix:client:api:*"
            );
            let mut response = salvo::http::Response::new();
            MatrixError::new(error, "Reauthentication required").render(&mut response);
            assert_eq!(
                response.status_code,
                Some(salvo::http::StatusCode::UNAUTHORIZED)
            );
            let mut actual: serde_json::Value = response.take_json().await.unwrap();
            actual.as_object_mut().unwrap().remove("error");
            assert_eq!(actual, encoded);
        }
        let challenge: StepUpChallenge =
            from_value(json!({"acr_values":"mfa", "max_age":0, "scope":"openid"})).unwrap();
        assert_eq!(challenge.max_age, Some(Duration::ZERO));
        assert_eq!(to_value(StepUpChallenge::default()).unwrap(), json!({}));
    }

    #[test]
    fn step_up_rejects_invalid_challenge_values() {
        for key in ["org.matrix.msc4363.scope", "org.matrix.msc4363.acr_values"] {
            for value in [
                json!(""),
                json!(" leading"),
                json!("trailing "),
                json!("two  spaces"),
                json!("tab\tvalue"),
                json!("newline\nvalue"),
                json!("nul\0value"),
                json!(["openid"]),
            ] {
                assert!(from_value::<StepUpChallenge>(json!({key:value})).is_err());
            }
        }
        for value in [json!(-1), json!(1.5), json!("300")] {
            assert!(
                from_value::<StepUpChallenge>(json!({"org.matrix.msc4363.max_age":value})).is_err()
            );
        }
        for value in ["a\\b", "a\"b", "é"] {
            assert!(ScopeValues::parse(value).is_none());
        }
    }
}
