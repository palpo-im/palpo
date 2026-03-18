//! Pages module

pub mod login;
pub mod setup;
pub mod password_change;
pub mod dashboard;
pub mod config;
pub mod config_toml_editor;
pub mod config_template;
pub mod config_import_export;
pub mod config_mode_switcher;
pub mod server_control;
pub mod matrix_admin_create;
pub mod users;
pub mod user_detail;
pub mod rooms;
pub mod room_detail;
pub mod federation;
pub mod media;
pub mod user_media_stats;
pub mod appservices;
pub mod server_status;
pub mod server_commands;
pub mod logs;

#[cfg(test)]
mod config_search_test;

#[cfg(test)]
mod config_feedback_test;

#[cfg(test)]
mod user_management_test;

pub use login::LoginPage;
pub use setup::SetupWizardPage;
pub use password_change::PasswordChangePage;
pub use dashboard::AdminDashboard;
pub use config::ConfigManager;
pub use config_toml_editor::ConfigTomlEditorPage;
pub use config_template::ConfigTemplatePage;
pub use config_import_export::ConfigImportExportPage;
pub use config_mode_switcher::ConfigModeSwitcher;
pub use server_control::ServerControlPage;
pub use matrix_admin_create::MatrixAdminCreatePage;
pub use users::UserManager;
pub use user_detail::UserDetail;
pub use rooms::RoomManager;
pub use room_detail::RoomDetailPage;
pub use federation::FederationManager;
pub use media::MediaManager;
pub use user_media_stats::UserMediaStatsManager;
pub use appservices::AppserviceManager;
pub use server_status::ServerStatusPage;
pub use server_commands::ServerCommandsPage;
pub use logs::AuditLogs;