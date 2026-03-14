//! OAuth2 Authorization Server metadata endpoint for Matrix clients (MSC3861/MSC2965)

use serde::Serialize;

use crate::core::MatrixError;
use crate::routing::prelude::*;

/// GET /_matrix/client/v1/auth_metadata
/// GET /_matrix/client/unstable/org.matrix.msc2965/auth_metadata
///
/// Returns the authorization server metadata that Element X uses
/// to discover OIDC capabilities.
#[endpoint]
pub async fn auth_metadata() -> JsonResult<AuthMetadataResponse> {
    let conf = config::get();
    let oidc = conf
        .enabled_oidc()
        .ok_or_else(|| MatrixError::not_found("OIDC not enabled"))?;

    if !oidc.enable_auth_server {
        return Err(MatrixError::not_found("Authorization server not enabled").into());
    }

    let base_url = conf.well_known_client();
    let base = base_url.trim_end_matches('/');

    json_ok(build_auth_metadata(base))
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
