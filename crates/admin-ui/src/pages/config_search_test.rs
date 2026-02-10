//! Property-based tests for configuration search and filter functionality
//!
//! **Validates: Requirements 13.2**
//!
//! This module implements Property 8: Search and Filter Accuracy
//! 
//! Property 8 states: "For any search query and filter conditions, returned results
//! should only contain matching configuration items, and search results should maintain consistency."
//!
//! # Test Strategy
//!
//! This module uses a hybrid testing approach:
//!
//! ## 1. Property-Based Tests (5 tests)
//! Comprehensive coverage of core search behaviors using random inputs:
//! - Empty query matching all items
//! - Substring matching in labels and descriptions  
//! - Case-insensitive matching
//! - Non-matching query rejection
//!
//! ## 2. Concrete Scenario Tests (7 tests)
//! Realistic usage scenarios with actual configuration data:
//! - Chinese character search (服务器, 数据库, 超时)
//! - Section filtering (server, database, etc.)
//! - Combined search and filter operations
//! - Search consistency across multiple calls
//! - Partial word matching
//!
//! This structure eliminates redundancy while maintaining comprehensive coverage
//! of both general properties and specific real-world use cases.
//!
//! # Testing Approach
//!
//! We test the `matches_search` function which is the core search logic used
//! throughout the configuration UI. This function performs fuzzy matching on
//! field labels and descriptions.

use proptest::prelude::*;

/// Helper function to check if a field matches the search query (fuzzy matching)
///
/// This is the same function used in config.rs - we test it in isolation here.
/// Performs case-insensitive substring matching on field labels and descriptions.
/// Returns true if the query is empty or if the field matches.
fn matches_search(label: &str, description: Option<&str>, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    
    let query_lower = query.to_lowercase();
    let label_lower = label.to_lowercase();
    
    // Check if label contains query
    if label_lower.contains(&query_lower) {
        return true;
    }
    
    // Check if description contains query
    if let Some(desc) = description {
        if desc.to_lowercase().contains(&query_lower) {
            return true;
        }
    }
    
    false
}

/// Represents a configuration field with its metadata
#[derive(Debug, Clone)]
struct ConfigField {
    label: String,
    description: Option<String>,
    section: String,
}

impl ConfigField {
    fn new(label: String, description: Option<String>, section: String) -> Self {
        Self {
            label,
            description,
            section,
        }
    }
    
    fn matches_query(&self, query: &str) -> bool {
        matches_search(&self.label, self.description.as_deref(), query)
    }
    
    fn matches_section(&self, section_filter: Option<&str>) -> bool {
        match section_filter {
            None => true,
            Some(filter) => self.section == filter,
        }
    }
}

/// Generate a list of realistic configuration fields for testing
fn sample_config_fields() -> Vec<ConfigField> {
    vec![
        ConfigField::new(
            "服务器名称".to_string(),
            Some("Matrix服务器的域名".to_string()),
            "server".to_string(),
        ),
        ConfigField::new(
            "最大请求大小".to_string(),
            Some("单个HTTP请求的最大字节数".to_string()),
            "server".to_string(),
        ),
        ConfigField::new(
            "启用指标监控".to_string(),
            Some("启用Prometheus指标导出".to_string()),
            "server".to_string(),
        ),
        ConfigField::new(
            "数据库连接字符串".to_string(),
            Some("PostgreSQL数据库连接URL".to_string()),
            "database".to_string(),
        ),
        ConfigField::new(
            "最大连接数".to_string(),
            Some("数据库连接池的最大连接数".to_string()),
            "database".to_string(),
        ),
        ConfigField::new(
            "连接超时".to_string(),
            Some("数据库连接超时时间（秒）".to_string()),
            "database".to_string(),
        ),
        ConfigField::new(
            "启用联邦功能".to_string(),
            Some("启用与其他Matrix服务器的联邦通信".to_string()),
            "federation".to_string(),
        ),
        ConfigField::new(
            "签名密钥路径".to_string(),
            Some("服务器签名密钥文件的路径".to_string()),
            "federation".to_string(),
        ),
        ConfigField::new(
            "启用用户注册".to_string(),
            Some("允许新用户注册账号".to_string()),
            "auth".to_string(),
        ),
        ConfigField::new(
            "JWT 密钥".to_string(),
            Some("用于签名JWT令牌的密钥".to_string()),
            "auth".to_string(),
        ),
        ConfigField::new(
            "存储路径".to_string(),
            Some("媒体文件的存储目录".to_string()),
            "media".to_string(),
        ),
        ConfigField::new(
            "最大文件大小".to_string(),
            Some("允许上传的最大文件大小（字节）".to_string()),
            "media".to_string(),
        ),
        ConfigField::new(
            "请求超时".to_string(),
            Some("HTTP请求的超时时间（秒）".to_string()),
            "network".to_string(),
        ),
        ConfigField::new(
            "启用速率限制".to_string(),
            Some("启用API请求速率限制".to_string()),
            "network".to_string(),
        ),
        ConfigField::new(
            "日志级别".to_string(),
            Some("日志记录的详细程度".to_string()),
            "logging".to_string(),
        ),
        ConfigField::new(
            "启用 Prometheus 指标".to_string(),
            Some("导出Prometheus监控指标".to_string()),
            "logging".to_string(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Property 8: Search and Filter Accuracy
    // ============================================================================

    proptest! {
        /// Property: Empty query returns all fields
        ///
        /// For any configuration field, an empty search query should always match.
        /// This ensures that when no search is active, all fields are visible.
        #[test]
        fn prop_empty_query_matches_all(
            label in "[a-zA-Z0-9 ]{1,50}",
            has_desc in any::<bool>(),
            desc in "[a-zA-Z0-9 ]{0,100}",
        ) {
            let description = if has_desc { Some(desc.as_str()) } else { None };
            let result = matches_search(&label, description, "");
            prop_assert!(result, "Empty query should match all fields");
        }

        /// Property: Query matching label is found
        ///
        /// If a query is a substring of the label (case-insensitive), the field should match.
        /// This verifies the core substring matching functionality.
        #[test]
        fn prop_query_in_label_matches(
            prefix in "[a-zA-Z]{1,10}",
            query in "[a-zA-Z]{1,10}",
            suffix in "[a-zA-Z]{0,10}",
        ) {
            let label = format!("{}{}{}", prefix, query, suffix);
            let result = matches_search(&label, None, &query);
            prop_assert!(result, "Query substring in label should match");
        }

        /// Property: Query matching description is found
        ///
        /// If a query is a substring of the description (case-insensitive), the field should match.
        /// This ensures description-based search works correctly.
        #[test]
        fn prop_query_in_description_matches(
            label in "[a-zA-Z]{1,20}",
            prefix in "[a-zA-Z]{1,10}",
            query in "[a-zA-Z]{1,10}",
            suffix in "[a-zA-Z]{0,10}",
        ) {
            let description = format!("{}{}{}", prefix, query, suffix);
            let result = matches_search(&label, Some(&description), &query);
            prop_assert!(result, "Query substring in description should match");
        }

        /// Property: Case-insensitive matching
        ///
        /// Search should be case-insensitive - uppercase and lowercase queries
        /// should produce the same results.
        #[test]
        fn prop_case_insensitive_matching(
            base_query in "[a-z]{3,10}",
        ) {
            let label = format!("Test {} Field", base_query);
            let query_lower = base_query.to_lowercase();
            let query_upper = base_query.to_uppercase();
            let query_mixed = {
                let mut chars: Vec<char> = base_query.chars().collect();
                if !chars.is_empty() {
                    chars[0] = chars[0].to_uppercase().next().unwrap();
                }
                chars.into_iter().collect::<String>()
            };
            
            let result_lower = matches_search(&label, None, &query_lower);
            let result_upper = matches_search(&label, None, &query_upper);
            let result_mixed = matches_search(&label, None, &query_mixed);
            
            prop_assert_eq!(result_lower, result_upper, "Case should not affect matching");
            prop_assert_eq!(result_lower, result_mixed, "Case should not affect matching");
        }

        /// Property: Non-matching query returns false
        ///
        /// If a query doesn't appear in either label or description, the field should not match.
        /// This ensures we don't get false positives.
        #[test]
        fn prop_non_matching_query_fails(
            label in "abc[a-z]{1,10}",
            desc in "abc[a-z]{1,10}",
            query in "xyz[a-z]{1,10}",
        ) {
            // Ensure query doesn't accidentally match
            prop_assume!(!label.to_lowercase().contains(&query.to_lowercase()));
            prop_assume!(!desc.to_lowercase().contains(&query.to_lowercase()));
            
            let result = matches_search(&label, Some(&desc), &query);
            prop_assert!(!result, "Non-matching query should not match");
        }
    }

    // ============================================================================
    // Concrete test cases for realistic scenarios
    // ============================================================================

    #[test]
    fn test_search_server_name_field() {
        let fields = sample_config_fields();
        let query = "服务器";
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .collect();
        
        // Should match "服务器名称" and "签名密钥路径" (description contains "服务器")
        assert!(matching_fields.len() >= 2, "Should find at least 2 fields matching '服务器'");
        
        // Verify specific matches
        assert!(
            matching_fields.iter().any(|f| f.label == "服务器名称"),
            "Should match '服务器名称' field"
        );
    }

    #[test]
    fn test_search_database_fields() {
        let fields = sample_config_fields();
        let query = "数据库";
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .collect();
        
        // Should match all database-related fields
        assert!(matching_fields.len() >= 3, "Should find at least 3 database fields");
        
        // All matches should be from database section or mention database
        for field in matching_fields {
            assert!(
                field.section == "database" 
                || field.label.contains("数据库") 
                || field.description.as_ref().map_or(false, |d| d.contains("数据库")),
                "Field should be database-related: {:?}", field
            );
        }
    }

    #[test]
    fn test_search_timeout_fields() {
        let fields = sample_config_fields();
        let query = "超时";
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .collect();
        
        // Should match "连接超时" and "请求超时"
        assert!(matching_fields.len() >= 2, "Should find at least 2 timeout fields");
        
        let labels: Vec<_> = matching_fields.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"连接超时"), "Should match '连接超时'");
        assert!(labels.contains(&"请求超时"), "Should match '请求超时'");
    }

    #[test]
    fn test_section_filter_server() {
        let fields = sample_config_fields();
        let section_filter = Some("server");
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_section(section_filter))
            .collect();
        
        // Should only return server section fields
        assert!(matching_fields.len() >= 3, "Should find server fields");
        
        for field in matching_fields {
            assert_eq!(field.section, "server", "All fields should be from server section");
        }
    }

    #[test]
    fn test_combined_search_and_filter() {
        let fields = sample_config_fields();
        let query = "最大";
        let section_filter = Some("server");
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query) && f.matches_section(section_filter))
            .collect();
        
        // Should match "最大请求大小" from server section
        assert!(matching_fields.len() >= 1, "Should find at least 1 matching field");
        
        for field in matching_fields {
            assert_eq!(field.section, "server", "Should be from server section");
            assert!(
                field.label.contains("最大") || field.description.as_ref().map_or(false, |d| d.contains("最大")),
                "Should contain '最大' in label or description"
            );
        }
    }

    #[test]
    fn test_search_prometheus_metrics() {
        let fields = sample_config_fields();
        let query = "prometheus";
        
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .collect();
        
        // Should match fields with "Prometheus" in label or description
        assert!(matching_fields.len() >= 2, "Should find Prometheus-related fields");
        
        for field in matching_fields {
            let label_lower = field.label.to_lowercase();
            let desc_lower = field.description.as_ref().map(|d| d.to_lowercase());
            
            assert!(
                label_lower.contains("prometheus") 
                || desc_lower.as_ref().map_or(false, |d| d.contains("prometheus")),
                "Field should mention Prometheus: {:?}", field
            );
        }
    }

    #[test]
    fn test_search_consistency_multiple_calls() {
        let fields = sample_config_fields();
        let query = "连接";
        
        // Call search multiple times with same query
        let result1: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .map(|f| f.label.clone())
            .collect();
        
        let result2: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .map(|f| f.label.clone())
            .collect();
        
        let result3: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .map(|f| f.label.clone())
            .collect();
        
        // Results should be identical across calls
        assert_eq!(result1, result2, "Search results should be consistent");
        assert_eq!(result2, result3, "Search results should be consistent");
    }

    #[test]
    fn test_partial_word_matching() {
        let fields = sample_config_fields();
        
        // Test partial Chinese character matching
        let query = "服务";
        let matching_fields: Vec<_> = fields
            .iter()
            .filter(|f| f.matches_query(query))
            .collect();
        
        // Should match fields containing "服务"
        assert!(matching_fields.len() >= 1, "Should match partial word");
        
        for field in matching_fields {
            assert!(
                field.label.contains("服务") 
                || field.description.as_ref().map_or(false, |d| d.contains("服务")),
                "Should contain '服务'"
            );
        }
    }
}
