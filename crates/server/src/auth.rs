use salvo::oapi::ToParameters;
use serde::Deserialize;

use crate::appservice::RegistrationInfo;
use crate::core::MatrixError;
use crate::core::identifiers::*;
use crate::core::serde::default_false;
use crate::data::user::{DbUser, DbUserDevice};

#[derive(Clone, Debug)]
pub struct AuthedInfo {
    pub user: DbUser,
    pub user_device: DbUserDevice,
    pub access_token_id: Option<i64>,
    pub appservice: Option<RegistrationInfo>,
}
impl AuthedInfo {
    pub fn user(&self) -> &DbUser {
        &self.user
    }
    pub fn user_id(&self) -> &UserId {
        &self.user.id
    }
    pub fn device_id(&self) -> &DeviceId {
        &self.user_device.device_id
    }
    pub fn access_token_id(&self) -> Option<i64> {
        self.access_token_id
    }
    pub fn appservice(&self) -> Option<&RegistrationInfo> {
        self.appservice.as_ref()
    }
    pub fn is_delegated_auth(&self) -> bool {
        self.access_token_id.is_none() && self.appservice.is_none()
    }
    pub fn is_admin(&self) -> bool {
        self.user.is_admin
    }
}

#[derive(Debug, Clone, Deserialize, ToParameters)]
#[salvo(parameters(default_parameter_in = Query))]
pub struct AuthArgs {
    pub user_id: Option<String>,
    pub device_id: Option<String>,
    pub access_token: Option<String>,
    #[salvo(parameter(parameter_in = Header))]
    pub authorization: Option<String>,

    #[serde(default = "default_false")]
    pub from_appservice: bool,
}

impl AuthArgs {
    /// Whether this request should be authenticated as an access-token request.
    ///
    /// An Authorization header takes precedence over the deprecated query-string
    /// form. HTTP authentication schemes are case-insensitive.
    pub fn uses_access_token(&self) -> bool {
        self.authorization.as_deref().map_or_else(
            || self.access_token.is_some(),
            |authorization| {
                authorization
                    .split_once(' ')
                    .map_or(authorization, |(scheme, _)| scheme)
                    .eq_ignore_ascii_case("Bearer")
            },
        )
    }

    pub fn require_access_token(&self) -> Result<&str, MatrixError> {
        if let Some(authorization) = &self.authorization {
            let Some((scheme, token)) = authorization.split_once(' ') else {
                return Err(MatrixError::missing_token("Invalid Bearer token."));
            };
            let token = token.trim_start_matches(' ');
            if scheme.eq_ignore_ascii_case("Bearer")
                && !token.is_empty()
                && !token.chars().any(char::is_whitespace)
            {
                Ok(token)
            } else {
                Err(MatrixError::missing_token("Invalid Bearer token."))
            }
        } else if let Some(access_token) = self.access_token.as_deref() {
            Ok(access_token)
        } else {
            Err(MatrixError::missing_token("Token not found."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AuthArgs;

    fn auth_args(authorization: Option<&str>, access_token: Option<&str>) -> AuthArgs {
        AuthArgs {
            user_id: None,
            device_id: None,
            access_token: access_token.map(ToOwned::to_owned),
            authorization: authorization.map(ToOwned::to_owned),
            from_appservice: false,
        }
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        for scheme in ["Bearer", "bearer", "BEARER", "BeArEr"] {
            let args = auth_args(Some(&format!("{scheme} token")), None);
            assert!(args.uses_access_token());
            assert_eq!(args.require_access_token().unwrap(), "token");
        }
    }

    #[test]
    fn bearer_accepts_multiple_spaces_before_the_token() {
        let args = auth_args(Some("Bearer   token"), None);
        assert!(args.uses_access_token());
        assert_eq!(args.require_access_token().unwrap(), "token");
    }

    #[test]
    fn malformed_bearer_credentials_are_rejected() {
        for authorization in ["Bearer", "Bearer ", "Bearer token suffix", "Bearer\ttoken"] {
            let args = auth_args(Some(authorization), None);
            assert!(args.require_access_token().is_err(), "{authorization}");
        }
    }

    #[test]
    fn query_token_is_used_when_authorization_is_absent() {
        let args = auth_args(None, Some("query-token"));
        assert!(args.uses_access_token());
        assert_eq!(args.require_access_token().unwrap(), "query-token");
    }

    #[test]
    fn bearer_header_takes_precedence_over_query_token() {
        let args = auth_args(Some("Bearer header-token"), Some("query-token"));
        assert!(args.uses_access_token());
        assert_eq!(args.require_access_token().unwrap(), "header-token");
    }

    #[test]
    fn non_bearer_authorization_takes_precedence_over_query_token() {
        let args = auth_args(Some("X-Matrix origin=example.org"), Some("query-token"));
        assert!(!args.uses_access_token());
        assert!(args.require_access_token().is_err());
    }
}
