//! Pages module

pub mod login;
pub mod setup;
pub mod password_change;
pub mod dashboard;
pub mod config;
pub mod config_template;
pub mod config_import_export;
pub mod users;
pub mod rooms;
pub mod federation;
pub mod media;
pub mod appservices;
pub mod logs;

#[cfg(test)]
mod config_search_test;

#[cfg(test)]
mod config_feedback_test;

pub use login::LoginPage;
pub use setup::SetupWizardPage;
pub use password_change::PasswordChangePage;
pub use dashboard::AdminDashboard;
pub use config::ConfigManager;
pub use config_template::ConfigTemplatePage;
pub use config_import_export::ConfigImportExportPage;
pub use users::UserManager;
pub use rooms::RoomManager;
pub use federation::FederationManager;
pub use media::MediaManager;
pub use appservices::AppserviceManager;
pub use logs::AuditLogs;