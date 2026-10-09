//! Experimental MSC1763 room retention policy.
use salvo::oapi::ToSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::events::EmptyStateKey;
use crate::macros::EventContent;

/// Largest exact JSON integer permitted by Matrix.
pub const MAX_LIFETIME: u64 = (1 << 53) - 1;

/// The content of `org.matrix.msc1763.retention`, with the empty state key.
#[derive(ToSchema, Clone, Debug, Default, PartialEq, Eq, Serialize, EventContent)]
#[palpo_event(type = "org.matrix.msc1763.retention", kind = State, state_key_type = EmptyStateKey)]
pub struct RoomRetentionEventContent {
    /// Minimum lifetime in milliseconds; absence/null means no lower bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_lifetime: Option<u64>,
    /// Maximum lifetime in milliseconds; absence/null means no upper bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lifetime: Option<u64>,
}

impl RoomRetentionEventContent {
    /// Validate the JSON integer range and lifetime ordering.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .min_lifetime
            .into_iter()
            .chain(self.max_lifetime)
            .any(|value| value > MAX_LIFETIME)
        {
            return Err("Retention lifetime exceeds the Matrix integer range");
        }
        if self
            .min_lifetime
            .zip(self.max_lifetime)
            .is_some_and(|(min, max)| min > max)
        {
            return Err("Retention min_lifetime exceeds max_lifetime");
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for RoomRetentionEventContent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            min_lifetime: Option<u64>,
            max_lifetime: Option<u64>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let policy = Self {
            min_lifetime: wire.min_lifetime,
            max_lifetime: wire.max_lifetime,
        };
        policy.validate().map_err(serde::de::Error::custom)?;
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json};

    use super::*;

    #[test]
    fn retention_lifetime_boundaries_and_invalid_policies() {
        for policy in [
            json!({}),
            json!({"min_lifetime":null,"max_lifetime":null}),
            json!({"min_lifetime":0,"max_lifetime":0}),
            json!({"max_lifetime":MAX_LIFETIME}),
        ] {
            assert!(from_value::<RoomRetentionEventContent>(policy).is_ok());
        }
        for policy in [
            json!({"min_lifetime":2,"max_lifetime":1}),
            json!({"max_lifetime":MAX_LIFETIME+1}),
            json!({"max_lifetime":-1}),
            json!({"max_lifetime":1.5}),
            json!({"max_lifetime":"1000"}),
        ] {
            assert!(from_value::<RoomRetentionEventContent>(policy).is_err());
        }
    }
}
