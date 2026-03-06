/// Comprehensive tests for admin system types and error handling
///
/// This test suite provides extensive coverage of error types,
/// conversions, and display implementations.

use palpo_admin_server::types::*;

// ===== Error Display Tests =====

#[test]
fn test_admin_error_display_webui() {
    let err = AdminError::WebUIAdminAlreadyExists;
    assert_eq!(err.to_string(), "Web UI admin already exists");
    
    let err = AdminError::WebUIAdminNotFound;
    assert_eq!(err.to_string(), "Web UI admin not found");
    
    let err = AdminError::InvalidSessionToken;
    assert_eq!(err.to_string(), "Invalid session token");
    
    let err = AdminError::SessionExpired;
    assert_eq!(err.to_string(), "Session expired");
}

#[test]
fn test_admin_error_display_database() {
    let err = AdminError::DatabaseConnectionFailed("connection refused".to_string());
    assert!(err.to_string().contains("connection refused"));
    
    let err = AdminError::DatabaseQueryFailed("syntax error".to_string());
    assert!(err.to_string().contains("syntax error"));
    
    let err = AdminError::DatabaseUnavailable;
    assert_eq!(err.to_string(), "Database unavailable");
    
    let err = AdminError::DatabaseMigrationFailed("version mismatch".to_string());
    assert!(err.to_string().contains("version mismatch"));
}

#[test]
fn test_admin_error_display_migration() {
    let err = AdminError::LegacyCredentialsNotFound;
    assert_eq!(err.to_string(), "Legacy credentials not found");
    
    let err = AdminError::MigrationFailed("data corruption".to_string());
    assert!(err.to_string().contains("data corruption"));
    
    let err = AdminError::StorageUnavailable;
    assert_eq!(err.to_string(), "Storage unavailable");
}

#[test]
fn test_admin_error_display_server_control() {
    let err = AdminError::ServerNotRunning;
    assert_eq!(err.to_string(), "Server not running");
    
    let err = AdminError::ServerAlreadyRunning;
    assert_eq!(err.to_string(), "Server already running");
    
    let err = AdminError::ServerStartFailed("port in use".to_string());
    assert!(err.to_string().contains("port in use"));
    
    let err = AdminError::ServerStopFailed("timeout".to_string());
    assert!(err.to_string().contains("timeout"));
}

#[test]
fn test_admin_error_display_configuration() {
    let err = AdminError::InvalidDatabaseUrl;
    assert_eq!(err.to_string(), "Invalid database URL");
    
    let err = AdminError::InvalidServerName;
    assert_eq!(err.to_string(), "Invalid server name");
    
    let err = AdminError::InvalidPort;
    assert_eq!(err.to_string(), "Invalid port");
    
    let err = AdminError::TLSCertificateNotFound;
    assert_eq!(err.to_string(), "TLS certificate not found");
    
    let err = AdminError::TLSPrivateKeyNotFound;
    assert_eq!(err.to_string(), "TLS private key not found");
    
    let err = AdminError::ConfigValidationFailed("missing field".to_string());
    assert!(err.to_string().contains("missing field"));
}

#[test]
fn test_admin_error_display_matrix_admin() {
    let err = AdminError::MatrixAdminAlreadyExists;
    assert_eq!(err.to_string(), "Matrix admin already exists");
    
    let err = AdminError::MatrixAdminNotFound;
    assert_eq!(err.to_string(), "Matrix admin not found");
    
    let err = AdminError::InvalidCredentials;
    assert_eq!(err.to_string(), "Invalid credentials");
    
    let err = AdminError::NotAnAdmin;
    assert_eq!(err.to_string(), "Not an admin user");
}

#[test]
fn test_admin_error_display_password_policy() {
    let err = AdminError::PasswordTooShort(8);
    assert!(err.to_string().contains("8 characters"));
    assert!(err.to_string().contains("minimum 12"));
    
    let err = AdminError::MissingUppercase;
    assert!(err.to_string().contains("uppercase"));
    
    let err = AdminError::MissingLowercase;
    assert!(err.to_string().contains("lowercase"));
    
    let err = AdminError::MissingDigit;
    assert!(err.to_string().contains("digit"));
    
    let err = AdminError::MissingSpecialChar;
    assert!(err.to_string().contains("special character"));
    
    let err = AdminError::PasswordNotChanged;
    assert!(err.to_string().contains("different from current"));
    
    let err = AdminError::PasswordChangeRequired;
    assert!(err.to_string().contains("change required"));
}

#[test]
fn test_admin_error_display_security() {
    let err = AdminError::RateLimitExceeded;
    let msg = err.to_string();
    assert!(msg.contains("login attempts") || msg.contains("try again"), "Message: {}", msg);
    
    let err = AdminError::CSRFValidationFailed;
    assert!(err.to_string().contains("CSRF"));
}

#[test]
fn test_admin_error_display_general() {
    let err = AdminError::CredentialsNotFound;
    assert!(err.to_string().contains("not found"));
    
    let err = AdminError::AuditLogFailed("disk full".to_string());
    assert!(err.to_string().contains("disk full"));
    
    let err = AdminError::MatrixApiError("404".to_string());
    assert!(err.to_string().contains("404"));
    
    let err = AdminError::AdminStatusNotSet;
    assert!(err.to_string().contains("not set"));
    
    let err = AdminError::PasswordHashError("algorithm error".to_string());
    assert!(err.to_string().contains("algorithm error"));
    
    let err = AdminError::IoError("file not found".to_string());
    assert!(err.to_string().contains("file not found"));
    
    let err = AdminError::TomlError("parse error".to_string());
    assert!(err.to_string().contains("parse error"));
    
    let err = AdminError::HttpError("timeout".to_string());
    assert!(err.to_string().contains("timeout"));
    
    let err = AdminError::InvalidInput("bad format".to_string());
    assert!(err.to_string().contains("bad format"));
}

// ===== Error Conversion Tests =====

#[test]
fn test_error_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let admin_err: AdminError = io_err.into();
    
    match admin_err {
        AdminError::IoError(msg) => assert!(msg.contains("file not found")),
        _ => panic!("Expected IoError"),
    }
}

// ===== ServerStatus Tests =====

#[test]
fn test_server_status_equality() {
    assert_eq!(ServerStatus::NotStarted, ServerStatus::NotStarted);
    assert_eq!(ServerStatus::Running, ServerStatus::Running);
    assert_ne!(ServerStatus::Running, ServerStatus::Stopped);
}

#[test]
fn test_server_status_copy() {
    let status1 = ServerStatus::Running;
    let status2 = status1;
    assert_eq!(status1, status2);
}

#[test]
fn test_server_status_all_variants() {
    let statuses = vec![
        ServerStatus::NotStarted,
        ServerStatus::Starting,
        ServerStatus::Running,
        ServerStatus::Stopping,
        ServerStatus::Stopped,
        ServerStatus::Error,
    ];
    
    // Ensure all variants are distinct
    for (i, status1) in statuses.iter().enumerate() {
        for (j, status2) in statuses.iter().enumerate() {
            if i == j {
                assert_eq!(status1, status2);
            } else {
                assert_ne!(status1, status2);
            }
        }
    }
}

// ===== Error Debug Tests =====

#[test]
fn test_admin_error_debug() {
    let err = AdminError::InvalidCredentials;
    let debug_str = format!("{:?}", err);
    assert!(debug_str.contains("InvalidCredentials"));
}

#[test]
fn test_admin_error_debug_with_message() {
    let err = AdminError::DatabaseConnectionFailed("test error".to_string());
    let debug_str = format!("{:?}", err);
    assert!(debug_str.contains("DatabaseConnectionFailed"));
    assert!(debug_str.contains("test error"));
}

// ===== Error Categorization Tests =====

#[test]
fn test_error_is_authentication_error() {
    let auth_errors = vec![
        AdminError::InvalidCredentials,
        AdminError::InvalidSessionToken,
        AdminError::SessionExpired,
        AdminError::NotAnAdmin,
    ];
    
    for err in auth_errors {
        // These should all be authentication-related
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("invalid") || 
            msg.contains("expired") || 
            msg.contains("not an admin") ||
            msg.contains("token"),
            "Error message: {}", msg
        );
    }
}

#[test]
fn test_error_is_database_error() {
    let db_errors = vec![
        AdminError::DatabaseConnectionFailed("test".to_string()),
        AdminError::DatabaseQueryFailed("test".to_string()),
        AdminError::DatabaseUnavailable,
        AdminError::DatabaseMigrationFailed("test".to_string()),
    ];
    
    for err in db_errors {
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("database"), "Error message: {}", msg);
    }
}

#[test]
fn test_error_is_password_policy_error() {
    let password_errors = vec![
        AdminError::PasswordTooShort(8),
        AdminError::MissingUppercase,
        AdminError::MissingLowercase,
        AdminError::MissingDigit,
        AdminError::MissingSpecialChar,
        AdminError::PasswordNotChanged,
    ];
    
    for err in password_errors {
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("password") || 
            msg.contains("uppercase") || 
            msg.contains("lowercase") ||
            msg.contains("digit") ||
            msg.contains("special"),
            "Error message: {}", msg
        );
    }
}

// ===== Edge Case Tests =====

#[test]
fn test_error_with_empty_message() {
    let err = AdminError::DatabaseConnectionFailed("".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Database connection failed"));
}

#[test]
fn test_error_with_long_message() {
    let long_msg = "a".repeat(1000);
    let err = AdminError::DatabaseQueryFailed(long_msg.clone());
    let msg = err.to_string();
    assert!(msg.contains(&long_msg));
}

#[test]
fn test_error_with_special_characters() {
    let special_msg = "Error: <>&\"'";
    let err = AdminError::MatrixApiError(special_msg.to_string());
    let msg = err.to_string();
    assert!(msg.contains(special_msg));
}

// ===== Serialization Tests (for types that implement Serialize) =====

#[test]
fn test_server_status_serialization() {
    let status = ServerStatus::Running;
    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("Running"));
}

#[test]
fn test_server_status_deserialization() {
    let json = "\"Running\"";
    let status: ServerStatus = serde_json::from_str(json).unwrap();
    assert_eq!(status, ServerStatus::Running);
}

#[test]
fn test_server_status_round_trip() {
    let statuses = vec![
        ServerStatus::NotStarted,
        ServerStatus::Starting,
        ServerStatus::Running,
        ServerStatus::Stopping,
        ServerStatus::Stopped,
        ServerStatus::Error,
    ];
    
    for original in statuses {
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: ServerStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }
}

// ===== Error Consistency Tests =====

#[test]
fn test_error_messages_are_consistent() {
    // Ensure similar errors have consistent messaging
    let err1 = AdminError::WebUIAdminNotFound;
    let err2 = AdminError::MatrixAdminNotFound;
    
    assert!(err1.to_string().contains("not found"));
    assert!(err2.to_string().contains("not found"));
}

#[test]
fn test_error_messages_are_user_friendly() {
    // Ensure error messages are clear and actionable
    let errors = vec![
        AdminError::InvalidCredentials,
        AdminError::SessionExpired,
        AdminError::ServerNotRunning,
        AdminError::InvalidDatabaseUrl,
    ];
    
    for err in errors {
        let msg = err.to_string();
        // Messages should not be empty
        assert!(!msg.is_empty());
        // Messages should not contain debug symbols
        assert!(!msg.contains("{{"));
        assert!(!msg.contains("}}"));
    }
}

// ===== Error Trait Implementation Tests =====

#[test]
fn test_admin_error_implements_error_trait() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<AdminError>();
}

#[test]
fn test_admin_error_implements_display() {
    fn assert_display<T: std::fmt::Display>() {}
    assert_display::<AdminError>();
}

#[test]
fn test_admin_error_implements_debug() {
    fn assert_debug<T: std::fmt::Debug>() {}
    assert_debug::<AdminError>();
}
