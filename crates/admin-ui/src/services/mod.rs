//! API services module

pub mod auth;
pub mod audit;
pub mod audit_api;
pub mod config_api;
pub mod config_template_api;
pub mod config_import_export_api;
pub mod user_admin_api;
pub mod room_admin_api;
pub mod federation_admin_api;
pub mod media_admin_api;
pub mod appservice_admin_api;

pub use auth::*;
pub use audit::*;
pub use audit_api::*;
pub use config_api::*;
pub use config_template_api::*;
pub use config_import_export_api::*;
pub use user_admin_api::*;
pub use room_admin_api::*;
pub use federation_admin_api::*;
pub use media_admin_api::*;
pub use appservice_admin_api::*;