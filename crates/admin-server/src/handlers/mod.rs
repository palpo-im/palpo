/// HTTP Handlers for Admin Server
///
/// This module contains all HTTP request handlers for the admin server endpoints.

pub mod webui_admin;
pub mod server_config;
pub mod server_control;
pub mod matrix_admin;

pub use webui_admin::AppState;
