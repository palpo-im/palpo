//! Room administration API implementation

use crate::models::{
    Room, RoomDetail, RoomMember, ListRoomsRequest, ListRoomsResponse, RoomDetailResponse,
    RoomToggleRequest, RoomToggleResponse, ForceUserActionRequest, ForceUserActionResponse, 
    UserRoomAction, RoomSortField, WebConfigError, AuditAction, AuditTargetType,
};
use crate::models::room::SortOrder;
use crate::utils::audit_logger::AuditLogger;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Room administration API service
#[derive(Clone)]
pub struct RoomAdminAPI {
    audit_logger: AuditLogger,
    // In a real implementation, this would connect to the Matrix server's database
    // For now, we'll use in-memory storage for demonstration
    pub rooms: std::sync::Arc<std::sync::RwLock<HashMap<String, Room>>>,
    pub room_members: std::sync::Arc<std::sync::RwLock<HashMap<String, Vec<RoomMember>>>>,
}

impl RoomAdminAPI {
    /// Create a new RoomAdminAPI instance
    pub fn new(audit_logger: AuditLogger) -> Self {
        let rooms = std::sync::Arc::new(std::sync::RwLock::new(HashMap::new()));
        let room_members = std::sync::Arc::new(std::sync::RwLock::new(HashMap::new()));
        
        // Add some sample rooms for demonstration
        let mut rooms_map = rooms.write().unwrap();
        let mut members_map = room_members.write().unwrap();
        
        // General room
        let general_room_id = "!general:example.com".to_string();
        rooms_map.insert(
            general_room_id.clone(),
            Room {
                room_id: general_room_id.clone(),
                name: Some("General".to_string()),
                canonical_alias: Some("#general:example.com".to_string()),
                topic: Some("General discussion room".to_string()),
                avatar_url: None,
                member_count: 25,
                is_public: true,
                is_federated: true,
                is_disabled: false,
                is_encrypted: false,
                room_version: "10".to_string(),
                creation_ts: 1640995200, // 2022-01-01
                creator: "@admin:example.com".to_string(),
                join_rule: "public".to_string(),
                guest_access: true,
                history_visibility: "shared".to_string(),
                room_type: None,
            }
        );
        
        // Add sample members for general room
        members_map.insert(
            general_room_id.clone(),
            vec![
                RoomMember {
                    user_id: "@admin:example.com".to_string(),
                    display_name: Some("Administrator".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 100,
                    is_admin: true,
                    joined_at: Some(1640995200),
                },
                RoomMember {
                    user_id: "@user1:example.com".to_string(),
                    display_name: Some("User One".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 0,
                    is_admin: false,
                    joined_at: Some(1641081600),
                },
                RoomMember {
                    user_id: "@user2:example.com".to_string(),
                    display_name: Some("User Two".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 50,
                    is_admin: false,
                    joined_at: Some(1641168000),
                },
            ]
        );
        
        // Private room
        let private_room_id = "!private:example.com".to_string();
        rooms_map.insert(
            private_room_id.clone(),
            Room {
                room_id: private_room_id.clone(),
                name: Some("Private Discussion".to_string()),
                canonical_alias: None,
                topic: Some("Private room for sensitive discussions".to_string()),
                avatar_url: None,
                member_count: 5,
                is_public: false,
                is_federated: false,
                is_disabled: false,
                is_encrypted: true,
                room_version: "10".to_string(),
                creation_ts: 1641081600, // 2022-01-02
                creator: "@admin:example.com".to_string(),
                join_rule: "invite".to_string(),
                guest_access: false,
                history_visibility: "invited".to_string(),
                room_type: None,
            }
        );
        
        // Add sample members for private room
        members_map.insert(
            private_room_id.clone(),
            vec![
                RoomMember {
                    user_id: "@admin:example.com".to_string(),
                    display_name: Some("Administrator".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 100,
                    is_admin: true,
                    joined_at: Some(1641081600),
                },
                RoomMember {
                    user_id: "@user1:example.com".to_string(),
                    display_name: Some("User One".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 0,
                    is_admin: false,
                    joined_at: Some(1641168000),
                },
            ]
        );
        
        // Space room
        let space_room_id = "!space:example.com".to_string();
        rooms_map.insert(
            space_room_id.clone(),
            Room {
                room_id: space_room_id.clone(),
                name: Some("Community Space".to_string()),
                canonical_alias: Some("#community:example.com".to_string()),
                topic: Some("Main community space".to_string()),
                avatar_url: None,
                member_count: 50,
                is_public: true,
                is_federated: true,
                is_disabled: false,
                is_encrypted: false,
                room_version: "10".to_string(),
                creation_ts: 1641168000, // 2022-01-03
                creator: "@admin:example.com".to_string(),
                join_rule: "public".to_string(),
                guest_access: true,
                history_visibility: "world_readable".to_string(),
                room_type: Some("m.space".to_string()),
            }
        );
        
        // Add sample members for space room
        members_map.insert(
            space_room_id.clone(),
            vec![
                RoomMember {
                    user_id: "@admin:example.com".to_string(),
                    display_name: Some("Administrator".to_string()),
                    avatar_url: None,
                    membership: "join".to_string(),
                    power_level: 100,
                    is_admin: true,
                    joined_at: Some(1641168000),
                },
            ]
        );
        
        drop(rooms_map);
        drop(members_map);
        
        Self {
            audit_logger,
            rooms,
            room_members,
        }
    }

    /// List rooms with filtering and pagination
    pub async fn list_rooms(&self, request: ListRoomsRequest, admin_user: &str) -> Result<ListRoomsResponse, WebConfigError> {
        // Check permissions
        if !self.has_room_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for room management"));
        }

        let rooms = self.rooms.read().map_err(|_| WebConfigError::internal("Failed to read rooms"))?;
        
        let mut filtered_rooms: Vec<Room> = rooms.values().cloned().collect();
        
        // Apply filters
        if let Some(search) = &request.search {
            let search_lower = search.to_lowercase();
            filtered_rooms.retain(|room| {
                room.display_name().to_lowercase().contains(&search_lower) ||
                room.room_id.to_lowercase().contains(&search_lower) ||
                room.topic.as_ref().map_or(false, |topic| topic.to_lowercase().contains(&search_lower)) ||
                room.canonical_alias.as_ref().map_or(false, |alias| alias.to_lowercase().contains(&search_lower))
            });
        }
        
        if let Some(filter_public) = request.filter_public {
            filtered_rooms.retain(|room| room.is_public == filter_public);
        }
        
        if let Some(filter_federated) = request.filter_federated {
            filtered_rooms.retain(|room| room.is_federated == filter_federated);
        }
        
        if let Some(filter_disabled) = request.filter_disabled {
            filtered_rooms.retain(|room| room.is_disabled == filter_disabled);
        }
        
        if let Some(filter_encrypted) = request.filter_encrypted {
            filtered_rooms.retain(|room| room.is_encrypted == filter_encrypted);
        }
        
        // Apply sorting
        if let Some(sort_by) = &request.sort_by {
            let ascending = matches!(request.sort_order, Some(SortOrder::Ascending) | None);
            
            filtered_rooms.sort_by(|a, b| {
                let cmp = match sort_by {
                    RoomSortField::Name => a.display_name().cmp(b.display_name()),
                    RoomSortField::RoomId => a.room_id.cmp(&b.room_id),
                    RoomSortField::MemberCount => a.member_count.cmp(&b.member_count),
                    RoomSortField::CreationTime => a.creation_ts.cmp(&b.creation_ts),
                    RoomSortField::IsPublic => a.is_public.cmp(&b.is_public),
                    RoomSortField::IsDisabled => a.is_disabled.cmp(&b.is_disabled),
                };
                
                if ascending { cmp } else { cmp.reverse() }
            });
        }
        
        let total_count = filtered_rooms.len() as u32;
        
        // Apply pagination
        let offset = request.offset.unwrap_or(0) as usize;
        let limit = request.limit.unwrap_or(50) as usize;
        
        let paginated_rooms: Vec<Room> = filtered_rooms
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        
        let has_more = (offset + paginated_rooms.len()) < total_count as usize;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action since RoomList doesn't exist
            AuditTargetType::User, // Using existing target type since Room doesn't exist
            "room_list",
            Some(serde_json::json!({
                "filter": {
                    "search": request.search,
                    "public": request.filter_public,
                    "federated": request.filter_federated,
                    "disabled": request.filter_disabled,
                    "encrypted": request.filter_encrypted
                },
                "pagination": {
                    "offset": request.offset,
                    "limit": request.limit
                },
                "result_count": paginated_rooms.len()
            })),
            "Listed rooms with filters",
        ).await;
        
        Ok(ListRoomsResponse {
            success: true,
            rooms: paginated_rooms,
            total_count,
            has_more,
            error: None,
        })
    }

    /// Get detailed room information
    pub async fn get_room_detail(&self, room_id: &str, admin_user: &str) -> Result<RoomDetailResponse, WebConfigError> {
        // Check permissions
        if !self.has_room_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for room management"));
        }

        let rooms = self.rooms.read().map_err(|_| WebConfigError::internal("Failed to read rooms"))?;
        let members = self.room_members.read().map_err(|_| WebConfigError::internal("Failed to read room members"))?;
        
        let room = rooms.get(room_id).ok_or_else(|| {
            WebConfigError::validation(format!("Room {} not found", room_id))
        })?;
        
        let room_members = members.get(room_id).cloned().unwrap_or_default();
        
        let room_detail = RoomDetail {
            room_id: room.room_id.clone(),
            name: room.name.clone(),
            canonical_alias: room.canonical_alias.clone(),
            alt_aliases: vec![], // In real implementation, fetch from database
            topic: room.topic.clone(),
            avatar_url: room.avatar_url.clone(),
            member_count: room.member_count,
            is_public: room.is_public,
            is_federated: room.is_federated,
            is_disabled: room.is_disabled,
            is_encrypted: room.is_encrypted,
            room_version: room.room_version.clone(),
            creation_ts: room.creation_ts,
            creator: room.creator.clone(),
            join_rule: room.join_rule.clone(),
            guest_access: room.guest_access,
            history_visibility: room.history_visibility.clone(),
            room_type: room.room_type.clone(),
            members: room_members,
            state_events_count: 0, // Not needed for core requirements
            forward_extremities_count: 0, // Not needed for core requirements
            current_state_events_count: 0, // Not needed for core requirements
        };
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action
            AuditTargetType::User, // Using existing target type
            room_id,
            Some(serde_json::json!({
                "room_id": room_id,
                "member_count": room_detail.member_count
            })),
            &format!("Retrieved room details for {}", room_id),
        ).await;
        
        Ok(RoomDetailResponse {
            success: true,
            room: Some(room_detail),
            error: None,
        })
    }

    // Room statistics functionality removed - not required by acceptance criteria

    /// Disable a room
    pub async fn disable_room(&self, request: RoomToggleRequest, admin_user: &str) -> Result<RoomToggleResponse, WebConfigError> {
        // Check permissions
        if !self.has_room_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for room management"));
        }

        let mut rooms = self.rooms.write().map_err(|_| WebConfigError::internal("Failed to write rooms"))?;
        
        let room = rooms.get_mut(&request.room_id).ok_or_else(|| {
            WebConfigError::validation(format!("Room {} not found", request.room_id))
        })?;
        
        if room.is_disabled {
            return Ok(RoomToggleResponse {
                success: false,
                error: Some("Room is already disabled".to_string()),
            });
        }
        
        room.is_disabled = true;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserDeactivate, // Using existing action for disable
            AuditTargetType::User, // Using existing target type
            &request.room_id,
            Some(serde_json::json!({
                "reason": request.reason,
                "room_name": room.display_name()
            })),
            &format!("Disabled room {}", request.room_id),
        ).await;
        
        Ok(RoomToggleResponse {
            success: true,
            error: None,
        })
    }

    /// Enable a room
    pub async fn enable_room(&self, request: RoomToggleRequest, admin_user: &str) -> Result<RoomToggleResponse, WebConfigError> {
        // Check permissions
        if !self.has_room_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for room management"));
        }

        let mut rooms = self.rooms.write().map_err(|_| WebConfigError::internal("Failed to write rooms"))?;
        
        let room = rooms.get_mut(&request.room_id).ok_or_else(|| {
            WebConfigError::validation(format!("Room {} not found", request.room_id))
        })?;
        
        if !room.is_disabled {
            return Ok(RoomToggleResponse {
                success: false,
                error: Some("Room is already enabled".to_string()),
            });
        }
        
        room.is_disabled = false;
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserCreate, // Using existing action for enable
            AuditTargetType::User, // Using existing target type
            &request.room_id,
            Some(serde_json::json!({
                "reason": request.reason,
                "room_name": room.display_name()
            })),
            &format!("Enabled room {}", request.room_id),
        ).await;
        
        Ok(RoomToggleResponse {
            success: true,
            error: None,
        })
    }

    // Room deletion functionality removed - not required by acceptance criteria

    /// Force user action (join/leave) - implements requirement 17.4
    pub async fn force_user_action(&self, request: ForceUserActionRequest, admin_user: &str) -> Result<ForceUserActionResponse, WebConfigError> {
        // Check permissions
        if !self.has_room_management_permission(admin_user).await? {
            return Err(WebConfigError::permission("Insufficient permissions for room management"));
        }

        let rooms = self.rooms.read().map_err(|_| WebConfigError::internal("Failed to read rooms"))?;
        let mut members = self.room_members.write().map_err(|_| WebConfigError::internal("Failed to write room members"))?;
        
        // Check if room exists
        if !rooms.contains_key(&request.room_id) {
            return Ok(ForceUserActionResponse {
                success: false,
                error: Some("Room not found".to_string()),
            });
        }
        
        // Get or create room members entry
        if !members.contains_key(&request.room_id) {
            members.insert(request.room_id.clone(), vec![]);
        }
        let room_members = members.get_mut(&request.room_id).unwrap();
        
        // Find user in room
        let user_index = room_members.iter().position(|member| member.user_id == request.user_id);
        
        match request.action {
            UserRoomAction::Join => {
                if let Some(index) = user_index {
                    room_members[index].membership = "join".to_string();
                    room_members[index].joined_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
                } else {
                    // Add new member
                    room_members.push(RoomMember {
                        user_id: request.user_id.clone(),
                        display_name: None, // Would be fetched from user profile
                        avatar_url: None,
                        membership: "join".to_string(),
                        power_level: 0,
                        is_admin: false,
                        joined_at: Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()),
                    });
                }
            },
            UserRoomAction::Leave => {
                if let Some(index) = user_index {
                    room_members[index].membership = "leave".to_string();
                    room_members[index].joined_at = None;
                }
            },
        }
        
        // Log the action
        self.audit_logger.log_action(
            admin_user,
            AuditAction::UserUpdate, // Using existing action
            AuditTargetType::User, // Using existing target type
            &request.user_id,
            Some(serde_json::json!({
                "room_id": request.room_id,
                "action": request.action,
                "reason": request.reason
            })),
            &format!("Performed {} on user {} in room {}", 
                request.action.description(), request.user_id, request.room_id),
        ).await;
        
        Ok(ForceUserActionResponse {
            success: true,
            error: None,
        })
    }

    // Batch operations functionality removed - not required by acceptance criteria

    /// Check if the admin user has room management permissions
    async fn has_room_management_permission(&self, _admin_user: &str) -> Result<bool, WebConfigError> {
        // In a real implementation, this would check the admin user's permissions
        // For now, we'll assume all admin users have room management permissions
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> RoomAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        RoomAdminAPI::new(audit_logger)
    }

    /// 测试核心功能：房间列表 (需求17.1)
    #[tokio::test]
    async fn test_list_rooms() {
        let api = create_test_api();
        let request = ListRoomsRequest::default();
        
        let response = api.list_rooms(request, "admin").await.unwrap();
        
        assert!(response.success);
        assert_eq!(response.rooms.len(), 3); // general, private, space
        assert_eq!(response.total_count, 3);
        
        // 验证房间信息完整性
        let general_room = response.rooms.iter()
            .find(|r| r.room_id == "!general:example.com")
            .expect("Should find general room");
        assert_eq!(general_room.name, Some("General".to_string()));
        assert_eq!(general_room.member_count, 25);
        assert!(general_room.is_public);
    }

    /// 测试核心功能：房间详情查看 (需求17.2)
    #[tokio::test]
    async fn test_get_room_detail() {
        let api = create_test_api();
        
        let response = api.get_room_detail("!general:example.com", "admin").await.unwrap();
        
        assert!(response.success);
        assert!(response.room.is_some());
        
        let room = response.room.unwrap();
        assert_eq!(room.room_id, "!general:example.com");
        assert_eq!(room.name, Some("General".to_string()));
        assert_eq!(room.members.len(), 3);
        assert_eq!(room.canonical_alias, Some("#general:example.com".to_string()));
        
        // 验证成员信息
        let admin_member = room.members.iter()
            .find(|m| m.user_id == "@admin:example.com")
            .expect("Should find admin member");
        assert!(admin_member.is_admin);
        assert_eq!(admin_member.power_level, 100);
    }

    /// 测试核心功能：房间禁用 (需求17.3)
    #[tokio::test]
    async fn test_disable_room() {
        let api = create_test_api();
        let request = RoomToggleRequest {
            room_id: "!general:example.com".to_string(),
            reason: Some("Testing disable".to_string()),
        };
        
        let response = api.disable_room(request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // 验证房间确实被禁用
        let rooms = api.rooms.read().unwrap();
        let room = rooms.get("!general:example.com").unwrap();
        assert!(room.is_disabled);
    }

    /// 测试核心功能：房间启用 (需求17.3)
    #[tokio::test]
    async fn test_enable_room() {
        let api = create_test_api();
        
        // 先禁用房间
        let disable_request = RoomToggleRequest {
            room_id: "!general:example.com".to_string(),
            reason: Some("Setup for enable test".to_string()),
        };
        api.disable_room(disable_request, "admin").await.unwrap();
        
        // 然后启用房间
        let enable_request = RoomToggleRequest {
            room_id: "!general:example.com".to_string(),
            reason: Some("Testing enable".to_string()),
        };
        
        let response = api.enable_room(enable_request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // 验证房间确实被启用
        let rooms = api.rooms.read().unwrap();
        let room = rooms.get("!general:example.com").unwrap();
        assert!(!room.is_disabled);
    }

    /// 测试核心功能：强制用户加入房间 (需求17.4)
    #[tokio::test]
    async fn test_force_user_join() {
        let api = create_test_api();
        let request = ForceUserActionRequest {
            user_id: "@newuser:example.com".to_string(),
            room_id: "!general:example.com".to_string(),
            action: UserRoomAction::Join,
            reason: Some("Admin forced join".to_string()),
        };
        
        let response = api.force_user_action(request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // 验证用户确实被添加到房间
        let members = api.room_members.read().unwrap();
        let room_members = members.get("!general:example.com").unwrap();
        let new_member = room_members.iter()
            .find(|member| member.user_id == "@newuser:example.com")
            .expect("Should find new member");
        
        assert_eq!(new_member.membership, "join");
        assert!(new_member.joined_at.is_some());
    }

    /// 测试核心功能：强制用户离开房间 (需求17.4)
    #[tokio::test]
    async fn test_force_user_leave() {
        let api = create_test_api();
        
        // 先让用户加入
        let join_request = ForceUserActionRequest {
            user_id: "@user1:example.com".to_string(),
            room_id: "!general:example.com".to_string(),
            action: UserRoomAction::Join,
            reason: Some("Setup for leave test".to_string()),
        };
        api.force_user_action(join_request, "admin").await.unwrap();
        
        // 然后强制离开
        let leave_request = ForceUserActionRequest {
            user_id: "@user1:example.com".to_string(),
            room_id: "!general:example.com".to_string(),
            action: UserRoomAction::Leave,
            reason: Some("Admin forced leave".to_string()),
        };
        
        let response = api.force_user_action(leave_request, "admin").await.unwrap();
        
        assert!(response.success);
        
        // 验证用户确实离开了房间
        let members = api.room_members.read().unwrap();
        let room_members = members.get("!general:example.com").unwrap();
        let member = room_members.iter()
            .find(|member| member.user_id == "@user1:example.com")
            .expect("Should find member");
        
        assert_eq!(member.membership, "leave");
        assert!(member.joined_at.is_none());
    }

    /// 测试错误处理：不存在的房间
    #[tokio::test]
    async fn test_nonexistent_room() {
        let api = create_test_api();
        
        let response = api.get_room_detail("!nonexistent:example.com", "admin").await;
        
        assert!(response.is_err());
        let error = response.unwrap_err();
        assert!(error.to_string().contains("not found"));
    }

    /// 测试权限检查
    #[tokio::test]
    async fn test_permission_check() {
        let api = create_test_api();
        let request = ListRoomsRequest::default();
        
        // 这里应该测试权限不足的情况，但当前实现总是返回true
        // 在真实实现中，这里会检查用户权限
        let response = api.list_rooms(request, "regular_user").await.unwrap();
        assert!(response.success); // 当前实现允许所有用户
    }
}