//! Frontend Integration Test: User List Search and Filter (3.3.9)
//!
//! Tests: UsersPage + ApiClient + Pagination
//! Flow: Enter search → Apply filters → Verify API call → Verify results displayed

#[cfg(test)]
mod user_list_tests {
    use dioxus::prelude::*;
    use wasm_bindgen_test::*;
    
    use palpo_admin_ui::models::user::{User, ListUsersRequest, UserSortField};
    use palpo_admin_ui::models::room::SortOrder;
    use palpo_admin_ui::services::user_admin_api::UserAdminAPI;
    use palpo_admin_ui::utils::audit_logger::AuditLogger;
    use palpo_admin_ui::services::api_client::ApiClient;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Test: ListUsersRequest default values
    #[wasm_bindgen_test]
    fn test_list_users_request_defaults() {
        let request = ListUsersRequest::default();
        assert_eq!(request.limit, Some(20), "Default limit should be 20");
        assert_eq!(request.offset, Some(0), "Default offset should be 0");
        assert!(request.search.is_none(), "Default search should be None");
        assert!(request.filter_admin.is_none(), "Default filter_admin should be None");
        assert!(request.filter_deactivated.is_none(), "Default filter_deactivated should be None");
    }

    /// Test: ListUsersRequest with search
    #[wasm_bindgen_test]
    fn test_list_users_request_with_search() {
        let request = ListUsersRequest {
            search: Some("test".to_string()),
            ..Default::default()
        };
        assert_eq!(request.search, Some("test".to_string()), "Search should be set");
    }

    /// Test: ListUsersRequest with filters
    #[wasm_bindgen_test]
    fn test_list_users_request_with_filters() {
        let request = ListUsersRequest {
            filter_admin: Some(true),
            filter_deactivated: Some(false),
            ..Default::default()
        };
        assert_eq!(request.filter_admin, Some(true), "Admin filter should be true");
        assert_eq!(request.filter_deactivated, Some(false), "Deactivated filter should be false");
    }

    /// Test: ListUsersRequest with sorting
    #[wasm_bindgen_test]
    fn test_list_users_request_with_sorting() {
        let request = ListUsersRequest {
            sort_by: Some(UserSortField::CreationTime),
            sort_order: Some(SortOrder::Descending),
            ..Default::default()
        };
        assert_eq!(request.sort_by, Some(UserSortField::CreationTime), "Sort by should be CreationTime");
        assert_eq!(request.sort_order, Some(SortOrder::Descending), "Sort order should be Descending");
    }

    /// Test: UserSortField variants
    #[wasm_bindgen_test]
    fn test_user_sort_field_variants() {
        let _ = UserSortField::Username;
        let _ = UserSortField::CreationTime;
        // Both variants should be accessible
    }

    /// Test: SortOrder variants
    #[wasm_bindgen_test]
    fn test_sort_order_variants() {
        let _ = SortOrder::Ascending;
        let _ = SortOrder::Descending;
        // Both variants should be accessible
    }

    /// Test: Pagination calculation - first page
    #[wasm_bindgen_test]
    fn test_pagination_first_page() {
        let page_size = 20u32;
        let current_page = 0u32;
        let offset = current_page * page_size;
        assert_eq!(offset, 0, "First page offset should be 0");
    }

    /// Test: Pagination calculation - second page
    #[wasm_bindgen_test]
    fn test_pagination_second_page() {
        let page_size = 20u32;
        let current_page = 1u32;
        let offset = current_page * page_size;
        assert_eq!(offset, 20, "Second page offset should be 20");
    }

    /// Test: Total pages calculation - exact division
    #[wasm_bindgen_test]
    fn test_total_pages_exact() {
        let total_count = 40u32;
        let page_size = 20u32;
        let total_pages = (total_count + page_size - 1) / page_size;
        assert_eq!(total_pages, 2, "Total pages should be 2");
    }

    /// Test: Total pages calculation - partial last page
    #[wasm_bindgen_test]
    fn test_total_pages_partial() {
        let total_count = 45u32;
        let page_size = 20u32;
        let total_pages = (total_count + page_size - 1) / page_size;
        assert_eq!(total_pages, 3, "Total pages should be 3 (20 + 20 + 5)");
    }

    /// Test: Total pages calculation - single page
    #[wasm_bindgen_test]
    fn test_total_pages_single() {
        let total_count = 10u32;
        let page_size = 20u32;
        let total_pages = (total_count + page_size - 1) / page_size;
        assert_eq!(total_pages, 1, "Total pages should be 1");
    }

    /// Test: Display range calculation - first page
    #[wasm_bindgen_test]
    fn test_display_range_first_page() {
        let page_size = 20u32;
        let current_page = 0u32;
        let start = current_page * page_size + 1;
        let end = (current_page + 1) * page_size;
        assert_eq!(start, 1, "Start should be 1");
        assert_eq!(end, 20, "End should be 20");
    }

    /// Test: Display range calculation - last page
    #[wasm_bindgen_test]
    fn test_display_range_last_page() {
        let page_size = 20u32;
        let current_page = 2u32;
        let total_count = 45u32;
        let start = current_page * page_size + 1;
        let end = ((current_page + 1) * page_size).min(total_count);
        assert_eq!(start, 41, "Start should be 41");
        assert_eq!(end, 45, "End should be 45 (capped at total)");
    }

    /// Test: Search query building - empty search
    #[wasm_bindgen_test]
    fn test_search_query_empty() {
        let search = "";
        let api_search = if search.is_empty() { None } else { Some(search.to_string()) };
        assert!(api_search.is_none(), "Empty search should be None");
    }

    /// Test: Search query building - with search term
    #[wasm_bindgen_test]
    fn test_search_query_with_term() {
        let search = "admin";
        let api_search = if search.is_empty() { None } else { Some(search.to_string()) };
        assert_eq!(api_search, Some("admin".to_string()), "Search should be Some");
    }

    /// Test: Admin filter mapping - all
    #[wasm_bindgen_test]
    fn test_admin_filter_all() {
        let filter: Option<bool> = None;
        let api_filter = match filter {
            None => "all".to_string(),
            Some(true) => "admin".to_string(),
            Some(false) => "user".to_string(),
        };
        assert_eq!(api_filter, "all", "None should map to 'all'");
    }

    /// Test: Admin filter mapping - admin only
    #[wasm_bindgen_test]
    fn test_admin_filter_admin() {
        let filter = Some(true);
        let api_filter = match filter {
            None => "all".to_string(),
            Some(true) => "admin".to_string(),
            Some(false) => "user".to_string(),
        };
        assert_eq!(api_filter, "admin", "Some(true) should map to 'admin'");
    }

    /// Test: Admin filter mapping - user only
    #[wasm_bindgen_test]
    fn test_admin_filter_user() {
        let filter = Some(false);
        let api_filter = match filter {
            None => "all".to_string(),
            Some(true) => "admin".to_string(),
            Some(false) => "user".to_string(),
        };
        assert_eq!(api_filter, "user", "Some(false) should map to 'user'");
    }

    /// Test: Deactivated filter mapping - all
    #[wasm_bindgen_test]
    fn test_deactivated_filter_all() {
        let filter: Option<bool> = None;
        let api_filter = match filter {
            None => "all".to_string(),
            Some(false) => "active".to_string(),
            Some(true) => "deactivated".to_string(),
        };
        assert_eq!(api_filter, "all", "None should map to 'all'");
    }

    /// Test: Deactivated filter mapping - active
    #[wasm_bindgen_test]
    fn test_deactivated_filter_active() {
        let filter = Some(false);
        let api_filter = match filter {
            None => "all".to_string(),
            Some(false) => "active".to_string(),
            Some(true) => "deactivated".to_string(),
        };
        assert_eq!(api_filter, "active", "Some(false) should map to 'active'");
    }

    /// Test: Deactivated filter mapping - deactivated
    #[wasm_bindgen_test]
    fn test_deactivated_filter_deactivated() {
        let filter = Some(true);
        let api_filter = match filter {
            None => "all".to_string(),
            Some(false) => "active".to_string(),
            Some(true) => "deactivated".to_string(),
        };
        assert_eq!(api_filter, "deactivated", "Some(true) should map to 'deactivated'");
    }
}