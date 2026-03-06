/// Validation Utilities for Request Validation
///
/// This module provides common validation functions used across all handlers.
/// All validation functions return a Result<(), ValidationError> for composability.

use crate::types::AdminError;

/// Maximum length for user_id (Matrix user ID format)
pub const MAX_USER_ID_LENGTH: usize = 255;
/// Maximum length for displayname
pub const MAX_DISPLAYNAME_LENGTH: usize = 256;
/// Maximum length for username in availability check
pub const MAX_USERNAME_LENGTH: usize = 64;
/// Maximum limit for pagination
pub const MAX_PAGINATION_LIMIT: i64 = 100;
/// Default pagination limit
pub const DEFAULT_PAGINATION_LIMIT: i64 = 50;

/// Validation error with details
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// Validate Matrix user ID format
///
/// User IDs must:
/// - Start with '@'
/// - Contain a localpart and server name separated by ':'
/// - Localpart can contain letters, digits, and some special characters
/// - Server name must be a valid domain or IP address
///
/// Returns Ok(()) if valid, Err(ValidationError) otherwise.
pub fn validate_user_id(user_id: &str) -> Result<(), ValidationError> {
    if user_id.is_empty() {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: "User ID cannot be empty".to_string(),
        });
    }

    if user_id.len() > MAX_USER_ID_LENGTH {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: format!(
                "User ID exceeds maximum length of {} characters",
                MAX_USER_ID_LENGTH
            ),
        });
    }

    if !user_id.starts_with('@') {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: "User ID must start with '@'".to_string(),
        });
    }

    // Split into localpart and server
    let (localpart, server) = match user_id[1..].split_once(':') {
        Some((l, s)) => (l, s),
        None => {
            return Err(ValidationError {
                field: "user_id".to_string(),
                message: "User ID must contain a server name (localpart:server)".to_string(),
            });
        }
    };

    if localpart.is_empty() {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: "User ID localpart cannot be empty".to_string(),
        });
    }

    if server.is_empty() {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: "User ID server name cannot be empty".to_string(),
        });
    }

    // Validate localpart characters (Matrix spec: a-z, 0-9, and some special chars)
    let valid_localpart_chars = |c: char| -> bool {
        c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '=' || c == '/'
    };

    if !localpart.chars().all(valid_localpart_chars) {
        return Err(ValidationError {
            field: "user_id".to_string(),
            message: "User ID localpart contains invalid characters".to_string(),
        });
    }

    Ok(())
}

/// Validate username for availability check
pub fn validate_username(username: &str) -> Result<(), ValidationError> {
    if username.is_empty() {
        return Err(ValidationError {
            field: "username".to_string(),
            message: "Username cannot be empty".to_string(),
        });
    }

    if username.len() > MAX_USERNAME_LENGTH {
        return Err(ValidationError {
            field: "username".to_string(),
            message: format!(
                "Username exceeds maximum length of {} characters",
                MAX_USERNAME_LENGTH
            ),
        });
    }

    // Usernames should not contain special Matrix characters
    let invalid_chars = [':', '@', '/'];
    if username.chars().any(|c| invalid_chars.contains(&c)) {
        return Err(ValidationError {
            field: "username".to_string(),
            message: "Username contains invalid characters".to_string(),
        });
    }

    Ok(())
}

/// Validate pagination limit
pub fn validate_limit(limit: Option<i64>) -> Result<i64, ValidationError> {
    match limit {
        Some(l) if l < 1 => Err(ValidationError {
            field: "limit".to_string(),
            message: "Limit must be at least 1".to_string(),
        }),
        Some(l) if l > MAX_PAGINATION_LIMIT => Ok(MAX_PAGINATION_LIMIT),
        Some(l) => Ok(l),
        None => Ok(DEFAULT_PAGINATION_LIMIT),
    }
}

/// Validate offset for pagination
pub fn validate_offset(offset: Option<i64>) -> Result<i64, ValidationError> {
    match offset {
        Some(o) if o < 0 => Err(ValidationError {
            field: "offset".to_string(),
            message: "Offset cannot be negative".to_string(),
        }),
        Some(o) => Ok(o),
        None => Ok(0),
    }
}

/// Validate displayname
pub fn validate_displayname(displayname: Option<&str>) -> Result<(), ValidationError> {
    match displayname {
        Some(name) if name.len() > MAX_DISPLAYNAME_LENGTH => Err(ValidationError {
            field: "displayname".to_string(),
            message: format!(
                "Displayname exceeds maximum length of {} characters",
                MAX_DISPLAYNAME_LENGTH
            ),
        }),
        _ => Ok(()),
    }
}

/// Validate device ID format
pub fn validate_device_id(device_id: &str) -> Result<(), ValidationError> {
    if device_id.is_empty() {
        return Err(ValidationError {
            field: "device_id".to_string(),
            message: "Device ID cannot be empty".to_string(),
        });
    }

    // Device IDs are typically 10-20 uppercase alphanumeric characters
    if device_id.len() < 4 || device_id.len() > 50 {
        return Err(ValidationError {
            field: "device_id".to_string(),
            message: "Device ID has invalid length".to_string(),
        });
    }

    if !device_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(ValidationError {
            field: "device_id".to_string(),
            message: "Device ID contains invalid characters".to_string(),
        });
    }

    Ok(())
}

/// Validate room ID format
pub fn validate_room_id(room_id: &str) -> Result<(), ValidationError> {
    if room_id.is_empty() {
        return Err(ValidationError {
            field: "room_id".to_string(),
            message: "Room ID cannot be empty".to_string(),
        });
    }

    if !room_id.starts_with('!') {
        return Err(ValidationError {
            field: "room_id".to_string(),
            message: "Room ID must start with '!'".to_string(),
        });
    }

    if room_id.len() > MAX_USER_ID_LENGTH {
        return Err(ValidationError {
            field: "room_id".to_string(),
            message: "Room ID exceeds maximum length".to_string(),
        });
    }

    Ok(())
}

/// Validate session token format
pub fn validate_session_token(token: &str) -> Result<(), ValidationError> {
    if token.is_empty() {
        return Err(ValidationError {
            field: "token".to_string(),
            message: "Session token cannot be empty".to_string(),
        });
    }

    // Session tokens are typically UUIDs or long random strings
    if token.len() < 16 {
        return Err(ValidationError {
            field: "token".to_string(),
            message: "Session token is too short".to_string(),
        });
    }

    Ok(())
}

/// Validate rate limit parameters
pub fn validate_rate_limit_params(
    messages_per_second: Option<i64>,
    burst_count: Option<i64>,
) -> Result<(), ValidationError> {
    if let Some(mps) = messages_per_second {
        if mps < 0 {
            return Err(ValidationError {
                field: "messages_per_second".to_string(),
                message: "Messages per second cannot be negative".to_string(),
            });
        }
        if mps > 10000 {
            return Err(ValidationError {
                field: "messages_per_second".to_string(),
                message: "Messages per second exceeds maximum allowed (10000)".to_string(),
            });
        }
    }

    if let Some(burst) = burst_count {
        if burst < 0 {
            return Err(ValidationError {
                field: "burst_count".to_string(),
                message: "Burst count cannot be negative".to_string(),
            });
        }
        if burst > 100000 {
            return Err(ValidationError {
                field: "burst_count".to_string(),
                message: "Burst count exceeds maximum allowed (100000)".to_string(),
            });
        }
    }

    Ok(())
}

/// Validate threepid medium
pub fn validate_threepid_medium(medium: &str) -> Result<(), ValidationError> {
    let valid_mediums = ["email", "phone", "msisdn"];
    if !valid_mediums.contains(&medium) {
        return Err(ValidationError {
            field: "medium".to_string(),
            message: format!(
                "Invalid threepid medium. Must be one of: {}",
                valid_mediums.join(", ")
            ),
        });
    }

    Ok(())
}

/// Validate search query
pub fn validate_search_query(query: Option<&str>) -> Result<(), ValidationError> {
    match query {
        Some(q) if q.len() > 500 => Err(ValidationError {
            field: "search".to_string(),
            message: "Search query exceeds maximum length of 500 characters".to_string(),
        }),
        _ => Ok(()),
    }
}

/// Convert ValidationError to AdminError for consistent error handling
impl From<ValidationError> for AdminError {
    fn from(err: ValidationError) -> Self {
        AdminError::InvalidInput(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_user_id_valid() {
        assert!(validate_user_id("@alice:example.com").is_ok());
        assert!(validate_user_id("@bob:matrix.org").is_ok());
        assert!(validate_user_id("@user123:localhost").is_ok());
    }

    #[test]
    fn test_validate_user_id_invalid() {
        assert!(validate_user_id("").is_err());
        assert!(validate_user_id("alice").is_err()); // Missing @
        assert!(validate_user_id("@alice").is_err()); // Missing server
        assert!(validate_user_id("@:example.com").is_err()); // Empty localpart
        assert!(validate_user_id("@alice:").is_err()); // Empty server
    }

    #[test]
    fn test_validate_username() {
        assert!(validate_username("alice").is_ok());
        assert!(validate_username("alice123").is_ok());
        assert!(validate_username("").is_err());
    }

    #[test]
    fn test_validate_limit() {
        assert_eq!(validate_limit(Some(10)).unwrap(), 10);
        assert!(validate_limit(Some(0)).is_err()); // Should error for 0
        assert_eq!(validate_limit(None).unwrap(), 50); // Default
        assert_eq!(validate_limit(Some(200)).unwrap(), 100); // Clamped to max
    }
}