/// Business logic tests for admin system types and error handling
///
/// This test suite verifies critical business logic and error handling.
/// Implementation detail tests (Debug traits, subjective formatting) are excluded.

use palpo_admin_server::types::*;

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
