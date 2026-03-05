/// Threepid Handler - HTTP handlers for third-party identifier lookup
///
/// This module implements threepid (third-party identifier) API endpoints:
/// - Lookup user by threepid (email, phone, etc.)
/// - Get user's threepids
/// - Add/remove threepids
/// - Validate threepids
/// - Lookup user by external ID (SSO)

use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};

use crate::types::AdminError;
use crate::repositories::{ThreepidRepository, UserThreepid, UserExternalId};

/// Threepid lookup response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidLookupResponse {
    pub user_id: String,
    pub medium: String,
    pub address: String,
    pub validated: bool,
    pub validated_at: Option<i64>,
}

/// User threepids response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserThreepidsResponse {
    pub user_id: String,
    pub threepids: Vec<ThreepidInfo>,
}

/// Threepid info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreepidInfo {
    pub medium: String,
    pub address: String,
    pub validated: bool,
    pub validated_at: Option<i64>,
    pub added_at: i64,
}

/// Add threepid request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddThreepidRequest {
    pub medium: String,  // "email", "phone", etc.
    pub address: String,
}

/// External ID lookup response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdLookupResponse {
    pub user_id: String,
    pub auth_provider: String,
    pub external_id: String,
}

/// User external IDs response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserExternalIdsResponse {
    pub user_id: String,
    pub external_ids: Vec<ExternalIdInfo>,
}

/// External ID info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalIdInfo {
    pub auth_provider: String,
    pub external_id: String,
    pub created_at: i64,
}

/// Add external ID request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddExternalIdRequest {
    pub auth_provider: String,
    pub external_id: String,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Threepid handler configuration
pub struct ThreepidHandler<T: ThreepidRepository> {
    threepid_repo: T,
}

impl<T: ThreepidRepository> ThreepidHandler<T> {
    /// Create a new handler with the given repository
    pub fn new(threepid_repo: T) -> Self {
        Self { threepid_repo }
    }

    /// Lookup user by threepid (medium + address)
    pub async fn lookup_user_by_threepid(
        &self,
        path: web::Path<(String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (medium, address) = path.into_inner();
        let decoded_address = urlencoding::decode(&address).unwrap_or(address.clone());

        let result = self.threepid_repo.lookup_user_by_threepid(&medium, &decoded_address).await?;

        match result {
            Some(r) => Ok(HttpResponse::Ok().json(ThreepidLookupResponse {
                user_id: r.user_id,
                medium: r.medium,
                address: r.address,
                validated: r.validated,
                validated_at: r.validated_at,
            })),
            None => Ok(HttpResponse::NotFound().json(SuccessResponse {
                success: false,
                message: format!("No user found with threepid {}: {}", medium, decoded_address),
            })),
        }
    }

    /// Get all threepids for a user
    pub async fn get_user_threepids(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let threepids = self.threepid_repo.get_user_threepids(&user_id).await?;

        let threepid_info: Vec<ThreepidInfo> = threepids.iter().map(|t| ThreepidInfo {
            medium: t.medium.clone(),
            address: t.address.clone(),
            validated: t.validated_ts.is_some(),
            validated_at: t.validated_ts,
            added_at: t.added_ts,
        }).collect();

        Ok(HttpResponse::Ok().json(UserThreepidsResponse {
            user_id: user_id.to_string(),
            threepids: threepid_info,
        }))
    }

    /// Add a threepid for a user
    pub async fn add_threepid(
        &self,
        user_id: web::Path<String>,
        req: web::Json<AddThreepidRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let threepid = self.threepid_repo.add_threepid(&user_id, &req.medium, &req.address).await?;

        tracing::info!("Added threepid {}:{} for user {}", req.medium, req.address, user_id);

        Ok(HttpResponse::Created().json(ThreepidInfo {
            medium: threepid.medium,
            address: threepid.address,
            validated: threepid.validated_ts.is_some(),
            validated_at: threepid.validated_ts,
            added_at: threepid.added_ts,
        }))
    }

    /// Remove a threepid from a user
    pub async fn remove_threepid(
        &self,
        path: web::Path<(String, String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, medium, address) = path.into_inner();
        let decoded_address = urlencoding::decode(&address).unwrap_or(address.clone());

        self.threepid_repo.remove_threepid(&user_id, &medium, &decoded_address).await?;

        tracing::info!("Removed threepid {}:{} for user {}", medium, decoded_address, user_id);

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("Threepid {}:{} removed successfully", medium, decoded_address),
        }))
    }

    /// Validate a threepid
    pub async fn validate_threepid(
        &self,
        path: web::Path<(String, String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, medium, address) = path.into_inner();
        let decoded_address = urlencoding::decode(&address).unwrap_or(address.clone());

        self.threepid_repo.validate_threepid(&user_id, &medium, &decoded_address).await?;

        tracing::info!("Validated threepid {}:{} for user {}", medium, decoded_address, user_id);

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("Threepid {}:{} validated successfully", medium, decoded_address),
        }))
    }

    /// Lookup user by external ID (SSO)
    pub async fn lookup_user_by_external_id(
        &self,
        path: web::Path<(String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (provider, external_id) = path.into_inner();
        let decoded_external_id = urlencoding::decode(&external_id).unwrap_or(external_id.clone());

        let result = self.threepid_repo.lookup_user_by_external_id(&provider, &decoded_external_id).await?;

        match result {
            Some(r) => Ok(HttpResponse::Ok().json(ExternalIdLookupResponse {
                user_id: r.user_id,
                auth_provider: r.auth_provider,
                external_id: r.external_id,
            })),
            None => Ok(HttpResponse::NotFound().json(SuccessResponse {
                success: false,
                message: format!("No user found with external ID {}:{}", provider, decoded_external_id),
            })),
        }
    }

    /// Get all external IDs for a user
    pub async fn get_user_external_ids(&self, user_id: web::Path<String>) -> Result<HttpResponse, AdminError> {
        let external_ids = self.threepid_repo.get_user_external_ids(&user_id).await?;

        let external_id_info: Vec<ExternalIdInfo> = external_ids.iter().map(|e| ExternalIdInfo {
            auth_provider: e.auth_provider.clone(),
            external_id: e.external_id.clone(),
            created_at: e.created_ts,
        }).collect();

        Ok(HttpResponse::Ok().json(UserExternalIdsResponse {
            user_id: user_id.to_string(),
            external_ids: external_id_info,
        }))
    }

    /// Add an external ID for a user
    pub async fn add_external_id(
        &self,
        user_id: web::Path<String>,
        req: web::Json<AddExternalIdRequest>,
    ) -> Result<HttpResponse, AdminError> {
        let external_id = self.threepid_repo.add_external_id(&user_id, &req.auth_provider, &req.external_id).await?;

        tracing::info!("Added external ID {}:{} for user {}", req.auth_provider, req.external_id, user_id);

        Ok(HttpResponse::Created().json(ExternalIdInfo {
            auth_provider: external_id.auth_provider,
            external_id: external_id.external_id,
            created_at: external_id.created_ts,
        }))
    }

    /// Remove an external ID
    pub async fn remove_external_id(
        &self,
        path: web::Path<(String, String, String)>,
    ) -> Result<HttpResponse, AdminError> {
        let (user_id, provider, external_id) = path.into_inner();
        let decoded_external_id = urlencoding::decode(&external_id).unwrap_or(external_id.clone());

        self.threepid_repo.remove_external_id(&user_id, &provider, &decoded_external_id).await?;

        tracing::info!("Removed external ID {}:{} for user {}", provider, decoded_external_id, user_id);

        Ok(HttpResponse::Ok().json(SuccessResponse {
            success: true,
            message: format!("External ID {}:{} removed successfully", provider, decoded_external_id),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DieselThreepidRepository;
    use palpo_data::DieselPool;

    #[tokio::test]
    #[ignore]
    async fn test_lookup_user_by_threepid() {}

    #[tokio::test]
    #[ignore]
    async fn test_get_user_threepids() {}

    #[tokio::test]
    #[ignore]
    async fn test_lookup_user_by_external_id() {}
}