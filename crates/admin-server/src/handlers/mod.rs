/// HTTP Handlers for Admin Server
///
/// This module contains all HTTP request handlers for the admin server endpoints.

pub mod webui_admin;
pub mod server_config;
pub mod server_control;
pub mod server_status;
pub mod matrix_admin;
pub mod user_handler;
pub mod device_handler;
// NOTE: The following handlers are still disabled, pending rewrite to use PalpoClient
// pub mod session_handler;
// pub mod rate_limit_handler;
// pub mod media_handler;
// pub mod shadow_ban_handler;
// pub mod threepid_handler;
pub mod auth_middleware;
pub mod validation;
pub mod audit_logger;

pub use webui_admin::AppState;
pub use webui_admin::UserAppState;
