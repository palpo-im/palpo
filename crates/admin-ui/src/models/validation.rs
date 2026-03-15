//! Configuration validation models and utilities

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration validation result
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ConfigError>,
    pub warnings: Vec<ConfigWarning>,
}

/// Configuration error
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigError {
    pub field: String,
    pub message: String,
    pub code: String,
}

/// Configuration warning
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigWarning {
    pub field: String,
    pub message: String,
    pub code: String,
}

/// Validation request for a single field
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateFieldRequest {
    pub field: String,
    pub value: serde_json::Value,
    pub context: Option<HashMap<String, serde_json::Value>>,
}

/// Validation request for entire configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ValidateConfigRequest {
    pub config: serde_json::Value,
    pub check_dependencies: bool,
}

impl ValidationResult {
    /// Create a successful validation result
    pub fn success() -> Self {
        Self {
            valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    /// Create a validation result with errors
    pub fn with_errors(errors: Vec<ConfigError>) -> Self {
        Self {
            valid: errors.is_empty(),
            errors,
            warnings: vec![],
        }
    }

    /// Create a validation result with warnings
    pub fn with_warnings(warnings: Vec<ConfigWarning>) -> Self {
        Self {
            valid: true,
            errors: vec![],
            warnings,
        }
    }

    /// Add an error to the validation result
    pub fn add_error(&mut self, error: ConfigError) {
        self.errors.push(error);
        self.valid = false;
    }

    /// Add a warning to the validation result
    pub fn add_warning(&mut self, warning: ConfigWarning) {
        self.warnings.push(warning);
    }

    /// Check if validation has any issues
    pub fn has_issues(&self) -> bool {
        !self.errors.is_empty() || !self.warnings.is_empty()
    }
}

impl ConfigError {
    /// Create a new configuration error
    pub fn new(field: impl Into<String>, message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            code: code.into(),
        }
    }

    /// Create a required field error
    pub fn required_field(field: impl Into<String>) -> Self {
        let field = field.into();
        Self::new(
            field.clone(),
            format!("Field '{}' is required", field),
            "REQUIRED_FIELD",
        )
    }

    /// Create an invalid format error
    pub fn invalid_format(field: impl Into<String>, expected: impl Into<String>) -> Self {
        let field = field.into();
        let expected = expected.into();
        Self::new(
            field.clone(),
            format!("Field '{}' has invalid format. Expected: {}", field, expected),
            "INVALID_FORMAT",
        )
    }

    /// Create an invalid value error
    pub fn invalid_value(field: impl Into<String>, value: impl Into<String>, reason: impl Into<String>) -> Self {
        let field = field.into();
        Self::new(
            field.clone(),
            format!("Field '{}' has invalid value '{}': {}", field, value.into(), reason.into()),
            "INVALID_VALUE",
        )
    }

    /// Create a dependency error
    pub fn dependency_error(field: impl Into<String>, dependency: impl Into<String>) -> Self {
        let field = field.into();
        let dependency = dependency.into();
        Self::new(
            field.clone(),
            format!("Field '{}' depends on '{}' but it's not properly configured", field, dependency),
            "DEPENDENCY_ERROR",
        )
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl ConfigWarning {
    /// Create a new configuration warning
    pub fn new(field: impl Into<String>, message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
            code: code.into(),
        }
    }

    /// Create a security warning
    pub fn security_warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(field, message, "SECURITY_WARNING")
    }

    /// Create a performance warning
    pub fn performance_warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(field, message, "PERFORMANCE_WARNING")
    }

    /// Create a deprecation warning
    pub fn deprecation_warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(field, message, "DEPRECATION_WARNING")
    }
}