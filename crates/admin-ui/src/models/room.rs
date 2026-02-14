//! Room management models

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Room information for management
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Room {
    pub room_id: String,
    pub name: Option<String>,
    pub canonical_alias: Option<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub member_count: u64,
    pub is_public: bool,
    pub is_federated: bool,
    pub is_disabled: bool,
    pub is_encrypted: bool,
    pub room_version: String,
    pub creation_ts: u64,
    pub creator: String,
    pub join_rule: String,
    pub guest_access: bool,
    pub history_visibility: String,
    pub room_type: Option<String>,
}

/// Room member information
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoomMember {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub membership: String, // "join", "invite", "leave", "ban", "knock"
    pub power_level: i32,
    pub is_admin: bool,
    pub joined_at: Option<u64>,
}

/// Room list request with filtering and pagination
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListRoomsRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub search: Option<String>,
    pub filter_public: Option<bool>,
    pub filter_federated: Option<bool>,
    pub filter_disabled: Option<bool>,
    pub filter_encrypted: Option<bool>,
    pub sort_by: Option<RoomSortField>,
    pub sort_order: Option<SortOrder>,
}

/// Room list response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ListRoomsResponse {
    pub success: bool,
    pub rooms: Vec<Room>,
    pub total_count: u32,
    pub has_more: bool,
    pub error: Option<String>,
}

/// Room sort fields
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum RoomSortField {
    Name,
    RoomId,
    MemberCount,
    CreationTime,
    IsPublic,
    IsDisabled,
}

/// Sort order
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Room detail response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomDetailResponse {
    pub success: bool,
    pub room: Option<RoomDetail>,
    pub error: Option<String>,
}

/// Detailed room information
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomDetail {
    pub room_id: String,
    pub name: Option<String>,
    pub canonical_alias: Option<String>,
    pub alt_aliases: Vec<String>,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub member_count: u64,
    pub is_public: bool,
    pub is_federated: bool,
    pub is_disabled: bool,
    pub is_encrypted: bool,
    pub room_version: String,
    pub creation_ts: u64,
    pub creator: String,
    pub join_rule: String,
    pub guest_access: bool,
    pub history_visibility: String,
    pub room_type: Option<String>,
    pub members: Vec<RoomMember>,
    pub state_events_count: u64,
    pub forward_extremities_count: u64,
    pub current_state_events_count: u64,
}

/// Room enable/disable request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomToggleRequest {
    pub room_id: String,
    pub reason: Option<String>,
}

/// Room enable/disable response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomToggleResponse {
    pub success: bool,
    pub error: Option<String>,
}

// Room deletion functionality removed - not required by acceptance criteria

/// Force user join/leave request
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ForceUserActionRequest {
    pub user_id: String,
    pub room_id: String,
    pub action: UserRoomAction,
    pub reason: Option<String>,
}

/// User room actions - only Join and Leave are supported per requirement 17.4
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum UserRoomAction {
    Join,
    Leave,
}

/// Force user action response
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ForceUserActionResponse {
    pub success: bool,
    pub error: Option<String>,
}

// Room statistics functionality removed - not required by acceptance criteria

// Batch operations functionality removed - not required by acceptance criteria

impl Room {
    /// Check if room is currently active (not disabled and has members)
    pub fn is_active(&self) -> bool {
        !self.is_disabled && self.member_count > 0
    }

    /// Get room age in days since creation
    pub fn age_in_days(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        (now - self.creation_ts) / 86400 // 86400 seconds in a day
    }

    /// Get display name (name or canonical alias or room ID)
    pub fn display_name(&self) -> &str {
        self.name.as_deref()
            .or(self.canonical_alias.as_deref())
            .unwrap_or(&self.room_id)
    }

    /// Check if room is a space
    pub fn is_space(&self) -> bool {
        self.room_type.as_deref() == Some("m.space")
    }
}

impl RoomMember {
    /// Check if member is currently joined
    pub fn is_joined(&self) -> bool {
        self.membership == "join"
    }

    /// Check if member is banned
    pub fn is_banned(&self) -> bool {
        self.membership == "ban"
    }

    /// Check if member has admin privileges
    pub fn has_admin_privileges(&self) -> bool {
        self.is_admin || self.power_level >= 100
    }

    /// Get display name (display name or user ID)
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.user_id)
    }
}

impl Default for ListRoomsRequest {
    fn default() -> Self {
        Self {
            limit: Some(50),
            offset: Some(0),
            search: None,
            filter_public: None,
            filter_federated: None,
            filter_disabled: Some(false), // By default, don't show disabled rooms
            filter_encrypted: None,
            sort_by: Some(RoomSortField::Name),
            sort_order: Some(SortOrder::Ascending),
        }
    }
}

impl RoomSortField {
    /// Get human-readable description of the sort field
    pub fn description(&self) -> &'static str {
        match self {
            RoomSortField::Name => "Room Name",
            RoomSortField::RoomId => "Room ID",
            RoomSortField::MemberCount => "Member Count",
            RoomSortField::CreationTime => "Creation Time",
            RoomSortField::IsPublic => "Public Status",
            RoomSortField::IsDisabled => "Disabled Status",
        }
    }
}

impl SortOrder {
    /// Get human-readable description of the sort order
    pub fn description(&self) -> &'static str {
        match self {
            SortOrder::Ascending => "Ascending",
            SortOrder::Descending => "Descending",
        }
    }
}

impl UserRoomAction {
    /// Get human-readable description of the action
    pub fn description(&self) -> &'static str {
        match self {
            UserRoomAction::Join => "Force Join",
            UserRoomAction::Leave => "Force Leave",
        }
    }
}