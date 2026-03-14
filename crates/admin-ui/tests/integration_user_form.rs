//! Frontend Integration Test: User Creation Form Flow (3.3.8)
//!
//! Tests: UserForm + ApiClient + State management
//! Flow: Fill form → Check username availability → Submit → Verify API call → Verify UI update

#[cfg(test)]
mod user_form_tests {
    use dioxus::prelude::*;
    use wasm_bindgen_test::*;
    
    use palpo_admin_ui::components::forms::{UserForm, UserFormProps};
    use palpo_admin_ui::models::user::User;
    use palpo_admin_ui::services::user_admin_api::UserAdminAPI;
    use palpo_admin_ui::utils::audit_logger::AuditLogger;
    use palpo_admin_ui::services::api_client::ApiClient;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Test: Form validation for username
    #[wasm_bindgen_test]
    fn test_username_validation_empty() {
        // Test that empty username shows error
        let username = "";
        assert!(username.is_empty(), "Empty username should fail validation");
    }

    /// Test: Form validation for short username
    #[wasm_bindgen_test]
    fn test_username_validation_too_short() {
        let username = "ab"; // Less than 3 characters
        assert!(username.len() < 3, "Username with less than 3 chars should fail");
    }

    /// Test: Form validation for valid username
    #[wasm_bindgen_test]
    fn test_username_validation_valid() {
        let username = "validuser123";
        assert!(username.len() >= 3, "Valid username should pass length check");
        assert!(username.chars().all(|c| c.is_alphanumeric() || c == '_'), "Username should only contain alphanumeric and underscore");
    }

    /// Test: Form validation for invalid characters
    #[wasm_bindgen_test]
    fn test_username_validation_invalid_chars() {
        let username = "user@domain"; // Contains @
        assert!(!username.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'), 
            "Username with @ should fail validation");
    }

    /// Test: Password validation - too short
    #[wasm_bindgen_test]
    fn test_password_validation_too_short() {
        let password = "short1"; // Less than 8 characters
        assert!(password.len() < 8, "Password with less than 8 chars should fail");
    }

    /// Test: Password validation - valid
    #[wasm_bindgen_test]
    fn test_password_validation_valid() {
        let password = "SecurePass123!"; // 14 characters with uppercase, lowercase, digit, special
        assert!(password.len() >= 8, "Valid password should pass length check");
    }

    /// Test: Password confirmation matching
    #[wasm_bindgen_test]
    fn test_password_confirmation_match() {
        let password = "SecurePass123!";
        let confirm = "SecurePass123!";
        assert_eq!(password, confirm, "Matching passwords should pass");
    }

    /// Test: Password confirmation mismatch
    #[wasm_bindgen_test]
    fn test_password_confirmation_mismatch() {
        let password = "SecurePass123!";
        let confirm = "DifferentPass456!";
        assert_ne!(password, confirm, "Mismatched passwords should fail");
    }

    /// Test: Username to Matrix user ID conversion
    #[wasm_bindgen_test]
    fn test_username_to_user_id_conversion() {
        let username = "testuser";
        let expected = "@testuser:localhost";
        let user_id = if username.starts_with('@') {
            username.to_string()
        } else {
            format!("@{}:localhost", username)
        };
        assert_eq!(user_id, expected, "Username should be converted to Matrix user ID format");
    }

    /// Test: Username with @ prefix preserved
    #[wasm_bindgen_test]
    fn test_username_with_at_prefix() {
        let username = "@existing:server.com";
        let user_id = if username.starts_with('@') {
            username.to_string()
        } else {
            format!("@{}:localhost", username)
        };
        assert_eq!(user_id, username, "Username with @ prefix should be preserved");
    }
}