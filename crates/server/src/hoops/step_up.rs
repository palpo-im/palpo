use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::auth::ADMIN_SCOPE;
use super::introspection::{IntrospectionResult, OAuthScopes};
use crate::MatrixError;
use crate::config::DelegatedAuthConfig;
use crate::core::error::{ErrorKind, ScopeValues, StepUpChallenge};

pub(super) fn authorize_admin(
    scopes: &OAuthScopes,
    is_admin: bool,
    evidence: &IntrospectionResult,
    policy: &DelegatedAuthConfig,
    now: Duration,
) -> Result<(), MatrixError> {
    if !is_admin {
        return Err(MatrixError::forbidden(
            "Requires server admin privileges",
            None,
        ));
    }
    let age_ok = policy.admin_max_age.is_none_or(|max_age| {
        evidence
            .auth_time
            .and_then(|time| now.checked_sub(Duration::from_secs(time)))
            .is_some_and(|age| age <= max_age)
    });
    let assurance_ok = policy.admin_acr_values.as_ref().is_none_or(|values| {
        evidence
            .acr
            .as_deref()
            .is_some_and(|acr| values.iter().any(|value| value == acr))
    });
    if scopes.contains(ADMIN_SCOPE) && age_ok && assurance_ok {
        return Ok(());
    }
    // Device identity has already been checked before a challenge is issued.
    let device = scopes.device_id().expect("verified Matrix device");
    let mut required_scopes = format!("{ADMIN_SCOPE} urn:matrix:client:device:{device}");
    // Preserve ordinary API access when replacing the client's token after step-up.
    if scopes.has_matrix_api_scope() {
        required_scopes.push_str(" urn:matrix:client:api:*");
    }
    Err(MatrixError::new(
        ErrorKind::InsufficientUserAuthentication {
            challenge: StepUpChallenge {
                acr_values: policy.admin_acr_values.clone(),
                max_age: policy.admin_max_age,
                scope: Some(ScopeValues::parse(&required_scopes).expect("validated scopes")),
            },
        },
        "Additional authentication required to complete request",
    ))
}

pub(super) fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::AcrValues;

    #[test]
    fn step_up_scope_age_assurance_and_retry_policy() {
        let scopes =
            OAuthScopes::parse(&format!("{ADMIN_SCOPE} urn:matrix:client:device:D")).unwrap();
        let policy = DelegatedAuthConfig {
            admin_max_age: Some(Duration::from_secs(300)),
            admin_acr_values: AcrValues::parse("mfa pwd"),
            ..Default::default()
        };
        let mut evidence: IntrospectionResult = serde_json::from_value(
            serde_json::json!({"active":true, "auth_time":700, "acr":"mfa"}),
        )
        .unwrap();
        assert!(
            authorize_admin(&scopes, true, &evidence, &policy, Duration::from_secs(1000)).is_ok()
        );
        // Cached evidence is evaluated against request time, without resetting its age.
        assert!(
            authorize_admin(
                &scopes,
                true,
                &evidence,
                &policy,
                Duration::from_millis(1_000_001)
            )
            .is_err()
        );
        for time in [None, Some(699), Some(1001)] {
            evidence.auth_time = time;
            assert!(
                authorize_admin(&scopes, true, &evidence, &policy, Duration::from_secs(1000))
                    .is_err()
            );
        }
        evidence.auth_time = Some(1000);
        for acr in [None, Some("weak"), Some("mfa pwd")] {
            evidence.acr = acr.map(ToOwned::to_owned);
            assert!(
                authorize_admin(&scopes, true, &evidence, &policy, Duration::from_secs(1000))
                    .is_err()
            );
        }
        evidence.acr = Some("pwd".to_owned());
        assert!(
            authorize_admin(&scopes, true, &evidence, &policy, Duration::from_secs(1000)).is_ok()
        );
        let api = OAuthScopes::parse("urn:matrix:client:api:* urn:matrix:client:device:D").unwrap();
        assert!(
            authorize_admin(&api, true, &evidence, &policy, Duration::from_secs(1000)).is_err()
        );
        assert!(matches!(
            authorize_admin(
                &scopes,
                false,
                &evidence,
                &policy,
                Duration::from_secs(1000)
            )
            .unwrap_err()
            .kind,
            ErrorKind::Forbidden
        ));
    }
}
