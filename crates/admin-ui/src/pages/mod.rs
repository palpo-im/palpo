//! Pages module

pub mod login;
pub mod dashboard;
pub mod config;
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
pub use dashboard::AdminDashboard;
pub use config::ConfigManager;
pub use users::UserManager;
pub use rooms::RoomManager;
pub use federation::FederationManager;
pub use media::MediaManager;
pub use appservices::AppserviceManager;
pub use logs::AuditLogs;