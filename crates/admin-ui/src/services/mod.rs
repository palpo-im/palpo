//! API services module

pub mod api_client;
pub mod auth;
pub mod audit;
pub mod audit_api;
pub mod config;
pub mod config_api;
pub mod config_template_api;
pub mod config_import_export_api;
pub mod user_admin_api;
pub mod room_admin_api;
pub mod federation_admin_api;
pub mod media_admin_api;
pub mod appservice_admin_api;
pub mod server_control_api;

#[cfg(test)]
mod config_api_test;

pub use api_client::*;
pub use auth::*;
pub use audit::*;
pub use audit_api::*;
// Re-export ConfigService from config module
pub use config::ConfigService;
pub use config_api::*;
pub use config_template_api::*;
// Re-export all from config_import_export_api (includes ConfigFormat)
pub use config_import_export_api::*;
pub use user_admin_api::*;
pub use room_admin_api::*;
pub use federation_admin_api::*;
pub use media_admin_api::*;
pub use appservice_admin_api::*;
pub use server_control_api::*;