//! OAuth2 Authorization Server metadata endpoint for Matrix clients (MSC3861/MSC2965)

use serde::Serialize;

use crate::routing::prelude::*;

/// GET /_matrix/client/v1/auth_metadata
/// GET /_matrix/client/unstable/org.matrix.msc2965/auth_metadata
#[endpoint]
pub async fn auth_metadata() -> JsonResult<AuthMetadataResponse> {
    let _ = crate::routing::oauth2::require_auth_server()?;
    let base = config::get().well_known_client();
    json_ok(build_auth_metadata(base.trim_end_matches('/')))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthMetadataResponse {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub revocation_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub response_modes_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prompt_values_supported: Vec<String>,
}

pub fn build_auth_metadata(base: &str) -> AuthMetadataResponse {
    AuthMetadataResponse {
        issuer: format!("{}/", base),
        authorization_endpoint: format!("{}/oauth2/authorize", base),
        token_endpoint: format!("{}/oauth2/token", base),
        registration_endpoint: format!("{}/oauth2/register", base),
        revocation_endpoint: format!("{}/oauth2/revoke", base),
        response_types_supported: vec!["code".into()],
        response_modes_supported: vec!["query".into(), "fragment".into()],
        grant_types_supported: vec!["authorization_code".into(), "refresh_token".into()],
        code_challenge_methods_supported: vec!["S256".into()],
        token_endpoint_auth_methods_supported: vec!["none".into()],
        prompt_values_supported: vec!["create".into()],
    }
}
