//! Property-based tests for configuration form operation feedback consistency
//!
//! **Validates: Requirements 13.5**
//!
//! This module implements Property 9: Operation Feedback Consistency
//! 
//! Property 9 states: "For any system operation, successful operations should return
//! success feedback, and failed operations should return failure feedback with specific
//! error information."
//!
//! # Test Strategy
//!
//! This module uses a hybrid testing approach:
//!
//! ## 1. Property-Based Tests
//! Comprehensive coverage of feedback behaviors using random inputs:
//! - Save operations with valid/invalid configurations
//! - Validation operations with various error conditions
//! - API error responses with different status codes
//! - Feedback state transitions
//!
//! ## 2. Concrete Scenario Tests
//! Realistic usage scenarios with actual configuration operations:
//! - Successful save operations showing success messages
//! - Failed save operations showing error messages
//! - Validation errors displayed to users
//! - Feedback clearing on new operations
//!
//! # Testing Approach
//!
//! We test the feedback mechanisms used in the configuration UI:
//! - Success/error state management
//! - Error message formatting and display
//! - Feedback consistency across operations
//! - User-facing error messages

use proptest::prelude::*;

/// Represents the result of a configuration operation
#[derive(Debug, Clone, PartialEq)]
pub enum OperationResult {
    Success,
    ValidationError(Vec<String>),
    ApiError { status: u16, message: String },
    NetworkError(String),
}

impl OperationResult {
    /// Check if the operation was successful
    pub fn is_success(&self) -> bool {
        matches!(self, OperationResult::Success)
    }
    
    /// Check if the operation failed
    pub fn is_failure(&self) -> bool {
        !self.is_success()
    }
    
    /// Get error message if operation failed
    pub fn error_message(&self) -> Option<String> {
        match self {
            OperationResult::Success => None,
            OperationResult::ValidationError(errors) => {
                Some(format!("配置验证失败: {}", errors.join(", ")))
            }
            OperationResult::ApiError { status, message } => {
                Some(format!("API错误 ({}): {}", status, message))
            }
            OperationResult::NetworkError(msg) => {
                Some(format!("网络错误: {}", msg))
            }
        }
    }
    
    /// Get success message if operation succeeded
    pub fn success_message(&self) -> Option<String> {
        match self {
            OperationResult::Success => Some("配置已成功保存".to_string()),
            _ => None,
        }
    }
}

/// Represents the feedback state in the UI
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackState {
    pub save_success: bool,
    pub error_message: Option<String>,
    pub validation_errors: Vec<(String, String)>,
}

impl FeedbackState {
    pub fn new() -> Self {
        Self {
            save_success: false,
            error_message: None,
            validation_errors: Vec::new(),
        }
    }
    
    /// Update feedback state based on operation result
    pub fn update_from_result(&mut self, result: OperationResult) {
        match result {
            OperationResult::Success => {
                self.save_success = true;
                self.error_message = None;
                self.validation_errors.clear();
            }
            OperationResult::ValidationError(errors) => {
                self.save_success = false;
                self.error_message = Some(format!("配置验证失败: {}", errors.join(", ")));
                self.validation_errors = errors
                    .into_iter()
                    .enumerate()
                    .map(|(i, err)| (format!("field_{}", i), err))
                    .collect();
            }
            OperationResult::ApiError { status, message } => {
                self.save_success = false;
                self.error_message = Some(format!("API错误 ({}): {}", status, message));
                self.validation_errors.clear();
            }
            OperationResult::NetworkError(msg) => {
                self.save_success = false;
                self.error_message = Some(format!("网络错误: {}", msg));
                self.validation_errors.clear();
            }
        }
    }
    
    /// Check if feedback state is consistent with operation result
    pub fn is_consistent_with(&self, result: &OperationResult) -> bool {
        match result {
            OperationResult::Success => {
                self.save_success && self.error_message.is_none()
            }
            OperationResult::ValidationError(_) => {
                !self.save_success 
                && self.error_message.is_some() 
                && !self.validation_errors.is_empty()
            }
            OperationResult::ApiError { .. } | OperationResult::NetworkError(_) => {
                !self.save_success && self.error_message.is_some()
            }
        }
    }
    
    /// Clear all feedback
    pub fn clear(&mut self) {
        self.save_success = false;
        self.error_message = None;
        self.validation_errors.clear();
    }
}

/// Simulates a configuration save operation
pub fn simulate_save_operation(config_valid: bool, api_available: bool) -> OperationResult {
    if !config_valid {
        return OperationResult::ValidationError(vec![
            "服务器名称不能为空".to_string(),
            "数据库连接字符串格式无效".to_string(),
        ]);
    }
    
    if !api_available {
        return OperationResult::NetworkError("无法连接到服务器".to_string());
    }
    
    OperationResult::Success
}

/// Simulates a configuration validation operation
pub fn simulate_validation_operation(
    server_name_valid: bool,
    db_connection_valid: bool,
    jwt_secret_valid: bool,
) -> OperationResult {
    let mut errors = Vec::new();
    
    if !server_name_valid {
        errors.push("服务器名称格式无效".to_string());
    }
    
    if !db_connection_valid {
        errors.push("数据库连接字符串格式无效".to_string());
    }
    
    if !jwt_secret_valid {
        errors.push("JWT密钥长度不足".to_string());
    }
    
    if errors.is_empty() {
        OperationResult::Success
    } else {
        OperationResult::ValidationError(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Property 9: Operation Feedback Consistency
    // ============================================================================

    proptest! {
        /// Property: Successful operations always show success feedback
        ///
        /// When an operation succeeds, the feedback state must indicate success
        /// and must not show any error messages.
        #[test]
        fn prop_success_shows_success_feedback(
            _dummy in any::<u8>(), // Just to make proptest run multiple times
        ) {
            let result = OperationResult::Success;
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result.clone());
            
            prop_assert!(feedback.save_success, "Success operation should set save_success to true");
            prop_assert!(feedback.error_message.is_none(), "Success operation should clear error message");
            prop_assert!(feedback.validation_errors.is_empty(), "Success operation should clear validation errors");
            prop_assert!(feedback.is_consistent_with(&result), "Feedback should be consistent with result");
        }

        /// Property: Failed operations always show error feedback
        ///
        /// When an operation fails, the feedback state must indicate failure
        /// and must provide specific error information.
        #[test]
        fn prop_failure_shows_error_feedback(
            error_count in 1..5usize,
        ) {
            let errors: Vec<String> = (0..error_count)
                .map(|i| format!("错误 {}", i))
                .collect();
            
            let result = OperationResult::ValidationError(errors.clone());
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result.clone());
            
            prop_assert!(!feedback.save_success, "Failed operation should set save_success to false");
            prop_assert!(feedback.error_message.is_some(), "Failed operation should set error message");
            prop_assert!(!feedback.validation_errors.is_empty(), "Validation failure should populate validation_errors");
            prop_assert!(feedback.is_consistent_with(&result), "Feedback should be consistent with result");
        }

        /// Property: API errors provide status code and message
        ///
        /// When an API error occurs, the feedback must include both the HTTP status
        /// code and a descriptive error message.
        #[test]
        fn prop_api_error_includes_status_and_message(
            status in 400u16..600u16,
            message_suffix in "[a-zA-Z0-9 ]{5,20}",
        ) {
            let message = format!("错误: {}", message_suffix);
            let result = OperationResult::ApiError {
                status,
                message: message.clone(),
            };
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result.clone());
            
            prop_assert!(!feedback.save_success, "API error should set save_success to false");
            
            let error_msg = feedback.error_message.as_ref().unwrap();
            prop_assert!(error_msg.contains(&status.to_string()), "Error message should contain status code");
            prop_assert!(error_msg.contains(&message), "Error message should contain original message");
            prop_assert!(feedback.is_consistent_with(&result), "Feedback should be consistent with result");
        }

        /// Property: Network errors provide descriptive messages
        ///
        /// When a network error occurs, the feedback must provide a clear
        /// description of the network issue.
        #[test]
        fn prop_network_error_provides_description(
            error_detail in "[a-zA-Z0-9 ]{5,30}",
        ) {
            let result = OperationResult::NetworkError(error_detail.clone());
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result.clone());
            
            prop_assert!(!feedback.save_success, "Network error should set save_success to false");
            prop_assert!(feedback.error_message.is_some(), "Network error should set error message");
            
            let error_msg = feedback.error_message.as_ref().unwrap();
            prop_assert!(error_msg.contains(&error_detail), "Error message should contain error detail");
            prop_assert!(feedback.is_consistent_with(&result), "Feedback should be consistent with result");
        }

        /// Property: Feedback state transitions are consistent
        ///
        /// When feedback state changes from one result to another, the state
        /// should always reflect the most recent operation result.
        #[test]
        fn prop_feedback_transitions_are_consistent(
            first_success in any::<bool>(),
            second_success in any::<bool>(),
        ) {
            let mut feedback = FeedbackState::new();
            
            // First operation
            let first_result = if first_success {
                OperationResult::Success
            } else {
                OperationResult::ValidationError(vec!["错误1".to_string()])
            };
            feedback.update_from_result(first_result.clone());
            prop_assert!(feedback.is_consistent_with(&first_result), "Feedback should match first result");
            
            // Second operation
            let second_result = if second_success {
                OperationResult::Success
            } else {
                OperationResult::ApiError {
                    status: 500,
                    message: "服务器错误".to_string(),
                }
            };
            feedback.update_from_result(second_result.clone());
            prop_assert!(feedback.is_consistent_with(&second_result), "Feedback should match second result");
            
            // Feedback should reflect only the second operation
            if second_success {
                prop_assert!(feedback.save_success, "Should show success for second operation");
                prop_assert!(feedback.error_message.is_none(), "Should clear errors from first operation");
            } else {
                prop_assert!(!feedback.save_success, "Should show failure for second operation");
                prop_assert!(feedback.error_message.is_some(), "Should show error for second operation");
            }
        }

        /// Property: Validation errors are field-specific
        ///
        /// When validation fails, each error should be associated with a specific
        /// field, allowing targeted error display in the UI.
        #[test]
        fn prop_validation_errors_are_field_specific(
            error_count in 1..10usize,
        ) {
            let errors: Vec<String> = (0..error_count)
                .map(|i| format!("字段{}验证失败", i))
                .collect();
            
            let result = OperationResult::ValidationError(errors.clone());
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result);
            
            prop_assert_eq!(
                feedback.validation_errors.len(),
                error_count,
                "Should have one validation error per field"
            );
            
            // Each error should have a field identifier
            for (field, _message) in &feedback.validation_errors {
                prop_assert!(field.starts_with("field_"), "Field should have identifier");
            }
        }

        /// Property: Clear operation resets all feedback
        ///
        /// When feedback is cleared, all success and error states should be reset
        /// to their initial values.
        #[test]
        fn prop_clear_resets_all_feedback(
            initial_success in any::<bool>(),
            error_count in 0..5usize,
        ) {
            let mut feedback = FeedbackState::new();
            
            // Set some feedback state
            feedback.save_success = initial_success;
            if !initial_success {
                feedback.error_message = Some("某个错误".to_string());
                feedback.validation_errors = (0..error_count)
                    .map(|i| (format!("field_{}", i), format!("错误{}", i)))
                    .collect();
            }
            
            // Clear feedback
            feedback.clear();
            
            // All state should be reset
            prop_assert!(!feedback.save_success, "save_success should be false after clear");
            prop_assert!(feedback.error_message.is_none(), "error_message should be None after clear");
            prop_assert!(feedback.validation_errors.is_empty(), "validation_errors should be empty after clear");
        }
    }

    // ============================================================================
    // Concrete test cases for realistic scenarios
    // ============================================================================

    #[test]
    fn test_successful_save_shows_success_message() {
        let result = simulate_save_operation(true, true);
        let mut feedback = FeedbackState::new();
        
        feedback.update_from_result(result.clone());
        
        assert!(feedback.save_success, "Should show success");
        assert!(feedback.error_message.is_none(), "Should not show error");
        assert!(result.success_message().is_some(), "Should have success message");
        assert_eq!(
            result.success_message().unwrap(),
            "配置已成功保存",
            "Success message should be user-friendly"
        );
    }

    #[test]
    fn test_validation_failure_shows_specific_errors() {
        let result = simulate_save_operation(false, true);
        let mut feedback = FeedbackState::new();
        
        feedback.update_from_result(result.clone());
        
        assert!(!feedback.save_success, "Should not show success");
        assert!(feedback.error_message.is_some(), "Should show error message");
        assert!(!feedback.validation_errors.is_empty(), "Should have validation errors");
        
        let error_msg = feedback.error_message.unwrap();
        assert!(error_msg.contains("服务器名称"), "Error should mention server name");
        assert!(error_msg.contains("数据库连接"), "Error should mention database connection");
    }

    #[test]
    fn test_network_failure_shows_connection_error() {
        let result = simulate_save_operation(true, false);
        let mut feedback = FeedbackState::new();
        
        feedback.update_from_result(result.clone());
        
        assert!(!feedback.save_success, "Should not show success");
        assert!(feedback.error_message.is_some(), "Should show error message");
        
        let error_msg = feedback.error_message.unwrap();
        assert!(error_msg.contains("网络错误"), "Error should indicate network issue");
        assert!(error_msg.contains("无法连接"), "Error should mention connection failure");
    }

    #[test]
    fn test_multiple_validation_errors_all_displayed() {
        let result = simulate_validation_operation(false, false, false);
        let mut feedback = FeedbackState::new();
        
        feedback.update_from_result(result);
        
        assert!(!feedback.save_success, "Should not show success");
        assert_eq!(feedback.validation_errors.len(), 3, "Should have 3 validation errors");
        
        let error_msg = feedback.error_message.unwrap();
        assert!(error_msg.contains("服务器名称"), "Should mention server name error");
        assert!(error_msg.contains("数据库连接"), "Should mention database error");
        assert!(error_msg.contains("JWT密钥"), "Should mention JWT error");
    }

    #[test]
    fn test_partial_validation_errors() {
        let result = simulate_validation_operation(true, false, true);
        let mut feedback = FeedbackState::new();
        
        feedback.update_from_result(result);
        
        assert!(!feedback.save_success, "Should not show success");
        assert_eq!(feedback.validation_errors.len(), 1, "Should have 1 validation error");
        
        let error_msg = feedback.error_message.unwrap();
        assert!(error_msg.contains("数据库连接"), "Should mention database error");
        assert!(!error_msg.contains("服务器名称"), "Should not mention server name");
        assert!(!error_msg.contains("JWT密钥"), "Should not mention JWT");
    }

    #[test]
    fn test_api_error_status_codes() {
        let test_cases = vec![
            (400, "错误的请求"),
            (401, "未授权"),
            (403, "禁止访问"),
            (404, "未找到"),
            (500, "服务器内部错误"),
            (503, "服务不可用"),
        ];
        
        for (status, message) in test_cases {
            let result = OperationResult::ApiError {
                status,
                message: message.to_string(),
            };
            let mut feedback = FeedbackState::new();
            
            feedback.update_from_result(result);
            
            assert!(!feedback.save_success, "API error should not show success");
            let error_msg = feedback.error_message.unwrap();
            assert!(
                error_msg.contains(&status.to_string()),
                "Error should contain status code {}",
                status
            );
            assert!(
                error_msg.contains(message),
                "Error should contain message '{}'",
                message
            );
        }
    }

    #[test]
    fn test_feedback_cleared_on_new_operation() {
        let mut feedback = FeedbackState::new();
        
        // First operation fails
        let first_result = OperationResult::ValidationError(vec![
            "错误1".to_string(),
            "错误2".to_string(),
        ]);
        feedback.update_from_result(first_result);
        
        assert!(!feedback.save_success, "First operation should fail");
        assert!(feedback.error_message.is_some(), "Should have error message");
        assert_eq!(feedback.validation_errors.len(), 2, "Should have 2 errors");
        
        // Second operation succeeds
        let second_result = OperationResult::Success;
        feedback.update_from_result(second_result);
        
        assert!(feedback.save_success, "Second operation should succeed");
        assert!(feedback.error_message.is_none(), "Error message should be cleared");
        assert!(feedback.validation_errors.is_empty(), "Validation errors should be cleared");
    }

    #[test]
    fn test_error_message_formatting() {
        let result = OperationResult::ValidationError(vec![
            "字段A无效".to_string(),
            "字段B为空".to_string(),
            "字段C格式错误".to_string(),
        ]);
        
        let error_msg = result.error_message().unwrap();
        
        assert!(error_msg.starts_with("配置验证失败:"), "Should have proper prefix");
        assert!(error_msg.contains("字段A无效"), "Should contain first error");
        assert!(error_msg.contains("字段B为空"), "Should contain second error");
        assert!(error_msg.contains("字段C格式错误"), "Should contain third error");
        assert!(error_msg.contains(", "), "Errors should be comma-separated");
    }

    #[test]
    fn test_success_has_no_error_message() {
        let result = OperationResult::Success;
        
        assert!(result.error_message().is_none(), "Success should have no error message");
        assert!(result.success_message().is_some(), "Success should have success message");
    }

    #[test]
    fn test_failure_has_no_success_message() {
        let results = vec![
            OperationResult::ValidationError(vec!["错误".to_string()]),
            OperationResult::ApiError {
                status: 500,
                message: "错误".to_string(),
            },
            OperationResult::NetworkError("错误".to_string()),
        ];
        
        for result in results {
            assert!(result.success_message().is_none(), "Failure should have no success message");
            assert!(result.error_message().is_some(), "Failure should have error message");
        }
    }

    #[test]
    fn test_feedback_consistency_check() {
        // Test success consistency
        let mut feedback = FeedbackState::new();
        feedback.save_success = true;
        feedback.error_message = None;
        assert!(
            feedback.is_consistent_with(&OperationResult::Success),
            "Success feedback should be consistent"
        );
        
        // Test validation error consistency
        let mut feedback = FeedbackState::new();
        feedback.save_success = false;
        feedback.error_message = Some("错误".to_string());
        feedback.validation_errors = vec![("field".to_string(), "错误".to_string())];
        assert!(
            feedback.is_consistent_with(&OperationResult::ValidationError(vec!["错误".to_string()])),
            "Validation error feedback should be consistent"
        );
        
        // Test API error consistency
        let mut feedback = FeedbackState::new();
        feedback.save_success = false;
        feedback.error_message = Some("API错误".to_string());
        assert!(
            feedback.is_consistent_with(&OperationResult::ApiError {
                status: 500,
                message: "错误".to_string(),
            }),
            "API error feedback should be consistent"
        );
    }
}
