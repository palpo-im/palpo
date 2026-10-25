use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

use crate::core::RoomId;
use crate::core::client::retention::{LifetimeLimits, RetentionLimits};
use crate::core::events::room::retention::{MAX_LIFETIME, RoomRetentionEventContent as Policy};

/// Optional MSC1763 retention, disabled by default.
#[derive(Clone, Debug, Default)]
pub struct RetentionConfig {
    pub enable: bool,
    /// Operator default (`*`) and per-room overrides.
    pub policies: BTreeMap<String, Policy>,
    pub limits: RetentionLimits,
}

impl RetentionConfig {
    pub fn effective(&self, room_id: &RoomId, room_policy: Option<Policy>) -> Policy {
        if let Some(policy) = self.policies.get(room_id.as_str()) {
            return policy.clone();
        }
        let policy = match room_policy {
            Some(policy) => policy,
            None => return self.policies.get("*").cloned().unwrap_or_default(),
        };
        let mut effective = Policy {
            min_lifetime: clamp(policy.min_lifetime, self.limits.min_lifetime.as_ref()),
            max_lifetime: clamp(policy.max_lifetime, self.limits.max_lifetime.as_ref()),
        };
        // Independent clamps can cross even for an originally valid policy. The
        // upper lifetime takes precedence, unless the operator's lower bound
        // requires raising it; incompatible bounds are rejected at startup.
        if let Some((min, max)) = effective.min_lifetime.zip(effective.max_lifetime)
            && min > max
        {
            let floor = self
                .limits
                .min_lifetime
                .as_ref()
                .and_then(|limit| limit.min)
                .unwrap_or(0);
            effective.min_lifetime = Some(max.max(floor));
            effective.max_lifetime = Some(max.max(floor));
        }
        effective
    }

    pub(super) fn validate(&self) -> Result<(), &'static str> {
        for limits in [&self.limits.min_lifetime, &self.limits.max_lifetime]
            .into_iter()
            .flatten()
        {
            Policy {
                min_lifetime: limits.min,
                max_lifetime: limits.max,
            }
            .validate()?;
        }
        let lower = self.limits.min_lifetime.clone().unwrap_or_default();
        let upper = self.limits.max_lifetime.clone().unwrap_or_default();
        if lower.min.unwrap_or(0) > upper.max.unwrap_or(MAX_LIFETIME) {
            return Err("Retention lifetime bounds are incompatible");
        }
        for (room, policy) in &self.policies {
            if room != "*" && RoomId::parse(room).is_err() {
                return Err("Invalid retention policy room ID");
            }
            policy.validate()?;
            // Limits constrain properties present in operator policies. Unlike
            // a room state policy, an omitted operator property stays omitted.
            for (value, limits) in [
                (policy.min_lifetime, self.limits.min_lifetime.as_ref()),
                (policy.max_lifetime, self.limits.max_lifetime.as_ref()),
            ] {
                if let Some(value) = value
                    && clamp(Some(value), limits) != Some(value)
                {
                    return Err("Operator retention policies must comply with server limits");
                }
            }
        }
        Ok(())
    }
}

fn clamp(value: Option<u64>, limits: Option<&LifetimeLimits>) -> Option<u64> {
    let Some(limits) = limits else {
        return value;
    };
    value.or(limits.min).map(|value| {
        value
            .max(limits.min.unwrap_or(0))
            .min(limits.max.unwrap_or(MAX_LIFETIME))
    })
}

impl<'de> Deserialize<'de> for RetentionConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Wire {
            enable: bool,
            policies: BTreeMap<String, Policy>,
            limits: RetentionLimits,
        }
        let wire = Wire::deserialize(deserializer)?;
        let config = Self {
            enable: wire.enable,
            policies: wire.policies,
            limits: wire.limits,
        };
        config.validate().map_err(serde::de::Error::custom)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json};

    use super::*;

    #[test]
    fn retention_startup_validates_programmatic_configuration() {
        let mut server: super::super::ServerConfig = from_value(json!({
            "server_name":"retention.example", "db":{"url":"unused"}
        }))
        .unwrap();
        server.check().unwrap();
        server.retention.policies.insert(
            "*".to_owned(),
            Policy {
                min_lifetime: Some(2),
                max_lifetime: Some(1),
            },
        );
        assert!(
            server
                .check()
                .unwrap_err()
                .to_string()
                .contains("Retention")
        );
    }

    #[test]
    fn retention_operator_omissions_do_not_inherit_room_limits() {
        let config: RetentionConfig = from_value(json!({
            "policies":{"*":{"max_lifetime":15778800000u64}},
            "limits":{
                "min_lifetime":{"min":86400000,"max":172800000},
                "max_lifetime":{"min":7889400000u64,"max":15778800000u64}
            }
        }))
        .unwrap();
        let room = RoomId::parse("!room:example.org").unwrap();
        let default = config.effective(&room, None);
        assert_eq!(default.min_lifetime, None);
        assert_eq!(default.max_lifetime, Some(15778800000));
        assert_eq!(
            config
                .effective(&room, Some(Policy::default()))
                .min_lifetime,
            Some(86400000)
        );
    }

    #[test]
    fn retention_crossed_clamps_preserve_operator_bounds() {
        let room = RoomId::parse("!room:example.org").unwrap();
        for (limits, min, max, expected) in [
            (json!({"max_lifetime":{"max":100}}), 400, 500, 100),
            (
                json!({"min_lifetime":{"min":200},"max_lifetime":{"max":300}}),
                0,
                100,
                200,
            ),
        ] {
            let config: RetentionConfig = from_value(json!({"limits":limits})).unwrap();
            let effective = config.effective(
                &room,
                Some(Policy {
                    min_lifetime: Some(min),
                    max_lifetime: Some(max),
                }),
            );
            assert_eq!(effective.min_lifetime, Some(expected));
            assert_eq!(effective.max_lifetime, Some(expected));
            effective.validate().unwrap();
        }
    }

    #[test]
    fn retention_defaults_overrides_limits_and_empty_room_policy() {
        let config: RetentionConfig = from_value(json!({"policies":{"*":{"max_lifetime":1000},"!override:example.org":{"max_lifetime":2000}},"limits":{"max_lifetime":{"min":100,"max":3000}}})).unwrap();
        let room = RoomId::parse("!room:example.org").unwrap();
        assert!(!config.enable);
        assert_eq!(config.effective(&room, None).max_lifetime, Some(1000));
        assert_eq!(
            config
                .effective(&room, Some(Policy::default()))
                .max_lifetime,
            Some(100)
        );
        assert_eq!(
            config
                .effective(
                    &room,
                    Some(Policy {
                        max_lifetime: Some(10_000),
                        ..Default::default()
                    })
                )
                .max_lifetime,
            Some(3000)
        );
        let room = RoomId::parse("!override:example.org").unwrap();
        assert_eq!(
            config
                .effective(&room, Some(Policy::default()))
                .max_lifetime,
            Some(2000)
        );
        for invalid in [
            json!({"policies":{"invalid":{}}}),
            json!({"limits":{"max_lifetime":{"min":2,"max":1}}}),
            json!({"policies":{"*":{"max_lifetime":10}},"limits":{"max_lifetime":{"min":20}}}),
            json!({"limits":{"min_lifetime":{"min":100},"max_lifetime":{"max":10}}}),
        ] {
            assert!(from_value::<RetentionConfig>(invalid).is_err());
        }
    }
}
