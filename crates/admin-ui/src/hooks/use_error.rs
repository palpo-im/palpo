//! Error handling hook for Dioxus frontend

use dioxus::prelude::*;
use crate::models::error::{ApiError, WebConfigError};
use std::collections::VecDeque;

/// Maximum number of errors to keep in history
const MAX_ERROR_HISTORY: usize = 10;

/// Error notification with display properties
#[derive(Clone, Debug)]
pub struct ErrorNotification {
    pub id: uuid::Uuid,
    pub error: ApiError,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub dismissed: bool,
    pub auto_dismiss_after: Option<chrono::Duration>,
}

/// Error handler state
#[derive(Clone)]
pub struct ErrorHandler {
    pub errors: Signal<VecDeque<ErrorNotification>>,
}

impl ErrorHandler {
    /// Add a new error to the handler
    pub fn add_error(&mut self, error: ApiError) {
        let notification = ErrorNotification {
            id: uuid::Uuid::new_v4(),
            error,
            timestamp: chrono::Utc::now(),
            dismissed: false,
            auto_dismiss_after: Some(chrono::Duration::seconds(5)),
        };

        let mut errors = self.errors.write();
        errors.push_back(notification);

        // Keep only the most recent errors
        while errors.len() > MAX_ERROR_HISTORY {
            errors.pop_front();
        }
    }

    /// Add an error from WebConfigError
    pub fn add_web_config_error(&mut self, error: WebConfigError) {
        let api_error = ApiError {
            message: error.user_message(),
            status_code: Some(error.status_code()),
            error_code: Some(error.error_code().to_string()),
            details: None,
        };
        self.add_error(api_error);
    }

    /// Add a simple error message
    pub fn add_message(&mut self, message: impl Into<String>) {
        let api_error = ApiError::new(message);
        self.add_error(api_error);
    }

    /// Dismiss an error by ID
    pub fn dismiss_error(&mut self, id: uuid::Uuid) {
        let mut errors = self.errors.write();
        if let Some(notification) = errors.iter_mut().find(|n| n.id == id) {
            notification.dismissed = true;
        }
    }

    /// Clear all errors
    pub fn clear_all(&mut self) {
        self.errors.write().clear();
    }

    /// Get active (non-dismissed) errors
    pub fn active_errors(&self) -> Vec<ErrorNotification> {
        self.errors
            .read()
            .iter()
            .filter(|n| !n.dismissed)
            .cloned()
            .collect()
    }

    /// Get the most recent error
    pub fn latest_error(&self) -> Option<ErrorNotification> {
        self.errors
            .read()
            .iter()
            .filter(|n| !n.dismissed)
            .last()
            .cloned()
    }

    /// Check if there are any active errors
    pub fn has_errors(&self) -> bool {
        self.errors.read().iter().any(|n| !n.dismissed)
    }
}

/// Hook for error handling in Dioxus components
pub fn use_error_handler() -> ErrorHandler {
    let errors = use_signal(|| VecDeque::<ErrorNotification>::new());

    ErrorHandler { errors }
}

/// Utility functions for ErrorNotification
impl ErrorNotification {
    /// Get CSS class for error severity
    pub fn severity_class(&self) -> &'static str {
        if self.error.is_server_error() {
            "error-critical"
        } else if self.error.is_auth_error() || self.error.is_permission_error() {
            "error-warning"
        } else if self.error.is_client_error() {
            "error-info"
        } else {
            "error-default"
        }
    }

    /// Get icon for error type
    pub fn icon(&self) -> &'static str {
        if self.error.is_server_error() {
            "⚠️"
        } else if self.error.is_auth_error() {
            "🔒"
        } else if self.error.is_permission_error() {
            "🚫"
        } else {
            "ℹ️"
        }
    }

    /// Check if error should be auto-dismissed
    pub fn should_auto_dismiss(&self) -> bool {
        self.auto_dismiss_after.is_some() && !self.dismissed
    }
}