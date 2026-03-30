#[cfg(test)]
mod tests {
    use crate::palpo_client::{CreateOrUpdateUserRequest, ListUsersQuery, PalpoRateLimitConfig};

    #[test]
    fn test_list_users_query_serialization() {
        let query = ListUsersQuery {
            from: Some(0),
            limit: Some(50),
            search_term: Some("test".to_string()),
            guests: Some(false),
            deactivated: Some(false),
            admins: Some(false),
        };

        let json = serde_json::to_string(&query).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("50"));
    }

    #[test]
    fn test_create_user_request_serialization() {
        let req = CreateOrUpdateUserRequest {
            displayname: Some("Test User".to_string()),
            password: Some("password123".to_string()),
            admin: Some(false),
            deactivated: Some(false),
            avatar_url: None,
            user_type: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Test User"));
        assert!(json.contains("password123"));
    }

    #[test]
    fn test_rate_limit_config_serialization() {
        let config = PalpoRateLimitConfig {
            messages_per_second: Some(100),
            burst_count: Some(50),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("100"));
        assert!(json.contains("50"));
    }

    #[test]
    fn test_list_users_query_deserialization() {
        let json = r#"{"from":10,"limit":25,"search_term":"admin","guests":false}"#;
        let query: ListUsersQuery = serde_json::from_str(json).unwrap();
        
        assert_eq!(query.from, Some(10));
        assert_eq!(query.limit, Some(25));
        assert_eq!(query.search_term, Some("admin".to_string()));
    }

    #[test]
    fn test_create_user_request_deserialization() {
        let json = r#"{"displayname":"John","password":"pass123","admin":true}"#;
        let req: CreateOrUpdateUserRequest = serde_json::from_str(json).unwrap();
        
        assert_eq!(req.displayname, Some("John".to_string()));
        assert_eq!(req.password, Some("pass123".to_string()));
        assert_eq!(req.admin, Some(true));
    }

    #[test]
    fn test_rate_limit_config_deserialization() {
        let json = r#"{"messages_per_second":200,"burst_count":100}"#;
        let config: PalpoRateLimitConfig = serde_json::from_str(json).unwrap();
        
        assert_eq!(config.messages_per_second, Some(200));
        assert_eq!(config.burst_count, Some(100));
    }
}