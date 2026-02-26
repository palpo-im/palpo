/// Palpo Admin Server Library
///
/// This crate provides the admin server functionality for Palpo, implementing
/// the first tier of the two-tier admin system. It operates independently of
/// the Palpo Matrix server and provides:
///
/// - Web UI admin authentication (PostgreSQL-backed)
/// - Session management
/// - Server configuration management (planned)
/// - Server lifecycle control (planned)
/// - Database migrations for admin credentials

// Core types used across the admin server
pub mod types;
pub use types::{
    AdminError, CreateMatrixAdminResponse, ServerConfig, ServerStatus, SessionToken,
    WebUIAdminCredentials,
};

// Database migrations for Web UI admin credentials
pub mod migrations;
pub use migrations::MigrationRunner;

// Web UI authentication service (Tier 1 - independent of Palpo)
pub mod webui_auth_service;
pub use webui_auth_service::WebUIAuthService;

// Session manager for Web UI admin sessions
pub mod session_manager;
pub use session_manager::SessionManager;

// Migration service for legacy credential migration
pub mod migration_service;
pub use migration_service::{LegacyCredentials, MigrationService};

// Matrix admin creation service (Tier 2 - requires Palpo running)
pub mod matrix_admin_creation;
pub use matrix_admin_creation::{MatrixAdminClient, MatrixAdminCreationService, UserInfoResponse};

// Matrix authentication service (Tier 2 - Matrix standard login)
pub mod matrix_auth_service;
pub use matrix_auth_service::{AuthResult, AuthService};

// HTTP handlers for REST API endpoints
pub mod handlers;
pub use handlers::AppState;

// Server configuration API for managing Palpo server config
pub mod server_config;
pub use server_config::ServerConfigAPI;

// Server control API for managing Palpo server lifecycle
pub mod server_control;
pub use server_control::{ServerControlAPI, ServerStatusInfo};
