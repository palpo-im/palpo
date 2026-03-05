/// Repository module - exports all repository implementations
///
/// This module provides the data access layer for user management operations.
/// All repositories use direct PostgreSQL operations with Diesel ORM.

pub use user_repository::{UserRepository, DieselUserRepository, User, UserAttributes, CreateUserInput, UpdateUserInput, UserFilter, UserListResult, UserDetails};
pub use device_repository::{DeviceRepository, DieselDeviceRepository, Device, CreateDeviceInput, UpdateDeviceInput, DeviceFilter, DeviceListResult, DeviceWithSessions};
pub use session_repository::{SessionRepository, DieselSessionRepository, UserIp, SessionInfo, WhoisInfo, SessionFilter, SessionListResult};
pub use rate_limit_repository::{RateLimitRepository, DieselRateLimitRepository, UserRateLimitConfig, UpdateRateLimitInput};
pub use media_repository::{MediaRepository, DieselMediaRepository, MediaMetadata, MediaFilter, MediaListResult};
pub use shadow_ban_repository::{ShadowBanRepository, DieselShadowBanRepository, ShadowBanStatus};
pub use threepid_repository::{ThreepidRepository, DieselThreepidRepository, UserThreepid, UserExternalId, ThreepidLookupResult, ExternalIdLookupResult};

use palpo_data::DieselPool;

/// Repository factory for creating repository instances
pub struct RepositoryFactory {
    db_pool: DieselPool,
}

impl RepositoryFactory {
    pub fn new(db_pool: DieselPool) -> Self {
        Self { db_pool }
    }

    pub fn user_repository(&self) -> DieselUserRepository {
        DieselUserRepository::new(self.db_pool.clone())
    }

    pub fn device_repository(&self) -> DieselDeviceRepository {
        DieselDeviceRepository::new(self.db_pool.clone())
    }

    pub fn session_repository(&self) -> DieselSessionRepository {
        DieselSessionRepository::new(self.db_pool.clone())
    }

    pub fn rate_limit_repository(&self) -> DieselRateLimitRepository {
        DieselRateLimitRepository::new(self.db_pool.clone())
    }

    pub fn media_repository(&self) -> DieselMediaRepository {
        DieselMediaRepository::new(self.db_pool.clone())
    }

    pub fn shadow_ban_repository(&self) -> DieselShadowBanRepository {
        DieselShadowBanRepository::new(self.db_pool.clone())
    }

    pub fn threepid_repository(&self) -> DieselThreepidRepository {
        DieselThreepidRepository::new(self.db_pool.clone())
    }
}