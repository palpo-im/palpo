//! Room detail page component with tabbed interface

use dioxus::prelude::*;
use crate::app::Route;
use crate::models::room::{RoomDetail, RoomMember};
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Tab types for room detail page
#[derive(Clone, Copy, Debug, PartialEq)]
enum RoomDetailTab {
    BasicInfo,
    Members,
    StateEvents,
    Media,
    ForwardExtremities,
}

/// Room detail page component with tabbed interface
#[component]
pub fn RoomDetailPage(room_id: String) -> Element {
    // State management
    let mut room = use_signal(|| None::<RoomDetail>);
    let mut loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let mut active_tab = use_signal(|| RoomDetailTab::BasicInfo);
    let mut is_editing = use_signal(|| false);
    
    // Edit form state
    let mut edit_name = use_signal(String::new);
    let mut edit_topic = use_signal(String::new);
    let mut edit_avatar_url = use_signal(String::new);

    // Load room data effect (placeholder - will be implemented in Phase 3)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 3
        // For now, create a mock room for UI testing
        let mock_room = RoomDetail {
            room_id: room_id.clone(),
            name: Some("Test Room".to_string()),
            canonical_alias: Some("#test:example.com".to_string()),
            alt_aliases: vec![],
            topic: Some("This is a test room".to_string()),
            avatar_url: None,
            member_count: 5,
            is_public: true,
            is_federated: true,
            is_disabled: false,
            is_encrypted: true,
            room_version: "10".to_string(),
            creation_ts: 1640000000,
            creator: "@admin:example.com".to_string(),
            join_rule: "public".to_string(),
            guest_access: false,
            history_visibility: "shared".to_string(),
            room_type: None,
            members: vec![],
            state_events_count: 42,
            forward_extremities_count: 1,
            current_state_events_count: 15,
        };
        room.set(Some(mock_room));
        loading.set(false);
    });

    // Initialize edit form when room data loads
    use_effect(move || {
        if let Some(r) = room() {
            edit_name.set(r.name.clone().unwrap_or_default());
            edit_topic.set(r.topic.clone().unwrap_or_default());
            edit_avatar_url.set(r.avatar_url.clone().unwrap_or_default());
        }
    });

    rsx! {
        div { class: "space-y-6",
            // Header with back button
            div { class: "flex items-center gap-4",
                Link {
                    to: Route::Rooms {},
                    class: "inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                    "← 返回房间列表"
                }
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "房间详情" }
                    if let Some(r) = room() {
                        p { class: "mt-1 text-sm text-gray-500", "{r.room_id}" }
                    }
                }
            }

            // Loading state
            if loading() {
                div { class: "bg-white shadow rounded-lg p-12",
                    Spinner { size: "large".to_string(), message: Some("加载房间信息...".to_string()) }
                }
            } else if let Some(err) = error() {
                div { class: "bg-white shadow rounded-lg p-6",
                    ErrorMessage { message: err }
                }
            } else if let Some(r) = room() {
                // Tab navigation
                div { class: "bg-white shadow rounded-lg",
                    div { class: "border-b border-gray-200",
                        nav { class: "flex -mb-px",
                            TabButton {
                                label: "基本信息",
                                icon: "🏠",
                                active: active_tab() == RoomDetailTab::BasicInfo,
                                onclick: move |_| active_tab.set(RoomDetailTab::BasicInfo)
                            }
                            TabButton {
                                label: "成员",
                                icon: "👥",
                                active: active_tab() == RoomDetailTab::Members,
                                onclick: move |_| active_tab.set(RoomDetailTab::Members)
                            }
                            TabButton {
                                label: "状态事件",
                                icon: "📋",
                                active: active_tab() == RoomDetailTab::StateEvents,
                                onclick: move |_| active_tab.set(RoomDetailTab::StateEvents)
                            }
                            TabButton {
                                label: "媒体",
                                icon: "🖼️",
                                active: active_tab() == RoomDetailTab::Media,
                                onclick: move |_| active_tab.set(RoomDetailTab::Media)
                            }
                            TabButton {
                                label: "前沿终点",
                                icon: "🔗",
                                active: active_tab() == RoomDetailTab::ForwardExtremities,
                                onclick: move |_| active_tab.set(RoomDetailTab::ForwardExtremities)
                            }
                        }
                    }

                    // Tab content
                    div { class: "p-6",
                        match active_tab() {
                            RoomDetailTab::BasicInfo => rsx! {
                                BasicInfoTab {
                                    room: r.clone(),
                                    is_editing: is_editing(),
                                    edit_name: edit_name(),
                                    edit_topic: edit_topic(),
                                    edit_avatar_url: edit_avatar_url(),
                                    on_edit_toggle: move |_| is_editing.set(!is_editing()),
                                    on_name_change: move |value: String| edit_name.set(value),
                                    on_topic_change: move |value: String| edit_topic.set(value),
                                    on_avatar_url_change: move |value: String| edit_avatar_url.set(value),
                                    on_save: move |_| {
                                        // TODO: Implement save in Phase 3
                                        is_editing.set(false);
                                    },
                                    on_cancel: move |_| {
                                        is_editing.set(false);
                                        // Reset form
                                        if let Some(r) = room() {
                                            edit_name.set(r.name.clone().unwrap_or_default());
                                            edit_topic.set(r.topic.clone().unwrap_or_default());
                                            edit_avatar_url.set(r.avatar_url.clone().unwrap_or_default());
                                        }
                                    },
                                    on_delete: move |_| {
                                        // TODO: Implement delete in Phase 3
                                    },
                                    on_block: move |_| {
                                        // TODO: Implement block in Phase 3
                                    }
                                }
                            },
                            RoomDetailTab::Members => rsx! {
                                MembersTab {
                                    room_id: r.room_id.clone(),
                                    members: r.members.clone()
                                }
                            },
                            RoomDetailTab::StateEvents => rsx! {
                                StateEventsTab {
                                    room_id: r.room_id.clone()
                                }
                            },
                            RoomDetailTab::Media => rsx! {
                                MediaTab {
                                    room_id: r.room_id.clone()
                                }
                            },
                            RoomDetailTab::ForwardExtremities => rsx! {
                                ForwardExtremitiesTab {
                                    room_id: r.room_id.clone()
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// Tab button component
#[component]
fn TabButton(
    label: String,
    icon: String,
    active: bool,
    onclick: EventHandler<()>,
) -> Element {
    let base_class = "group inline-flex items-center px-4 py-4 border-b-2 font-medium text-sm";
    let active_class = if active {
        "border-blue-500 text-blue-600"
    } else {
        "border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300"
    };

    rsx! {
        button {
            class: "{base_class} {active_class}",
            onclick: move |_| onclick.call(()),
            span { class: "mr-2", "{icon}" }
            "{label}"
        }
    }
}

/// Basic info tab component
#[component]
fn BasicInfoTab(
    room: RoomDetail,
    is_editing: bool,
    edit_name: String,
    edit_topic: String,
    edit_avatar_url: String,
    on_edit_toggle: EventHandler<()>,
    on_name_change: EventHandler<String>,
    on_topic_change: EventHandler<String>,
    on_avatar_url_change: EventHandler<String>,
    on_save: EventHandler<()>,
    on_cancel: EventHandler<()>,
    on_delete: EventHandler<()>,
    on_block: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            // Room avatar and basic info
            div { class: "flex items-start gap-6",
                // Avatar
                div { class: "flex-shrink-0",
                    if let Some(avatar_url) = &room.avatar_url {
                        img {
                            class: "h-24 w-24 rounded",
                            src: "{avatar_url}",
                            alt: "{room.name.as_ref().unwrap_or(&room.room_id)}"
                        }
                    } else {
                        div { class: "h-24 w-24 rounded bg-gray-300 flex items-center justify-center text-gray-600 text-3xl font-semibold",
                            "{room.name.as_ref().and_then(|n| n.chars().next()).unwrap_or('R').to_uppercase()}"
                        }
                    }
                }

                // Room info
                div { class: "flex-1",
                    if is_editing {
                        // Edit form
                        div { class: "space-y-4",
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "房间名称" }
                                input {
                                    r#type: "text",
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                    value: "{edit_name}",
                                    placeholder: "房间名称",
                                    oninput: move |evt| on_name_change.call(evt.value())
                                }
                            }
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "房间主题" }
                                textarea {
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                    rows: "3",
                                    value: "{edit_topic}",
                                    placeholder: "房间主题描述",
                                    oninput: move |evt| on_topic_change.call(evt.value())
                                }
                            }
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "头像 URL" }
                                input {
                                    r#type: "text",
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                    value: "{edit_avatar_url}",
                                    placeholder: "mxc://example.com/avatar",
                                    oninput: move |evt| on_avatar_url_change.call(evt.value())
                                }
                            }
                            div { class: "flex gap-2",
                                button {
                                    class: "inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                                    onclick: move |_| on_save.call(()),
                                    "💾 保存"
                                }
                                button {
                                    class: "inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                                    onclick: move |_| on_cancel.call(()),
                                    "取消"
                                }
                            }
                        }
                    } else {
                        // Display mode
                        div { class: "space-y-3",
                            div {
                                h3 { class: "text-2xl font-bold text-gray-900",
                                    "{room.name.as_ref().unwrap_or(&room.room_id)}"
                                }
                                if let Some(alias) = &room.canonical_alias {
                                    p { class: "text-sm text-gray-500", "{alias}" }
                                }
                            }
                            if let Some(topic) = &room.topic {
                                p { class: "text-sm text-gray-600", "{topic}" }
                            }
                            div { class: "flex flex-wrap gap-2",
                                if room.is_public {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                                        "🌐 公开"
                                    }
                                } else {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                                        "🔒 私有"
                                    }
                                }
                                if room.is_encrypted {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-100 text-blue-800",
                                        "🔐 已加密"
                                    }
                                }
                                if room.is_federated {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-purple-100 text-purple-800",
                                        "🌍 联邦"
                                    }
                                }
                                if room.is_disabled {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                                        "❌ 已禁用"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Room details
            if !is_editing {
                div { class: "border-t border-gray-200 pt-6",
                    h4 { class: "text-lg font-medium text-gray-900 mb-4", "详细信息" }
                    div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                        InfoItem { label: "房间ID", value: room.room_id.clone() }
                        InfoItem { label: "创建者", value: room.creator.clone() }
                        InfoItem { label: "成员数", value: format!("{}", room.member_count) }
                        InfoItem { label: "房间版本", value: room.room_version.clone() }
                        InfoItem { label: "加入规则", value: room.join_rule.clone() }
                        InfoItem { label: "历史可见性", value: room.history_visibility.clone() }
                        InfoItem { label: "访客访问", value: if room.guest_access { "允许".to_string() } else { "禁止".to_string() } }
                        InfoItem { label: "状态事件数", value: format!("{}", room.state_events_count) }
                        InfoItem { label: "前沿终点数", value: format!("{}", room.forward_extremities_count) }
                        InfoItem { label: "当前状态事件数", value: format!("{}", room.current_state_events_count) }
                        InfoItem { label: "创建时间", value: format_timestamp(room.creation_ts) }
                        if let Some(room_type) = &room.room_type {
                            InfoItem { label: "房间类型", value: room_type.clone() }
                        }
                    }
                }

                // Action buttons
                div { class: "border-t border-gray-200 pt-6",
                    div { class: "flex flex-wrap gap-3",
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                            onclick: move |_| on_edit_toggle.call(()),
                            "✏️ 编辑房间"
                        }
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-red-300 text-sm font-medium rounded-md text-red-700 bg-white hover:bg-red-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-red-500",
                            onclick: move |_| on_delete.call(()),
                            "🗑️ 删除房间"
                        }
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-red-300 text-sm font-medium rounded-md text-red-700 bg-white hover:bg-red-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-red-500",
                            onclick: move |_| on_block.call(()),
                            "🚫 删除并封禁"
                        }
                    }
                }
            }
        }
    }
}

/// Info item component for displaying key-value pairs
#[component]
fn InfoItem(label: String, value: String) -> Element {
    rsx! {
        div { class: "bg-gray-50 px-4 py-3 rounded-md",
            dt { class: "text-sm font-medium text-gray-500", "{label}" }
            dd { class: "mt-1 text-sm text-gray-900", "{value}" }
        }
    }
}

/// Members tab component (placeholder for Phase 3)
#[component]
fn MembersTab(room_id: String, members: Vec<RoomMember>) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 成员管理功能将在第三阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "👥" }
                p { class: "text-gray-500 text-lg", "房间成员列表" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示房间 {room_id} 的所有成员（头像、ID、显示名、访客状态、停用状态、锁定状态）"
                }
            }
        }
    }
}

/// State events tab component (placeholder for Phase 3)
#[component]
fn StateEventsTab(room_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 状态事件管理功能将在第三阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "📋" }
                p { class: "text-gray-500 text-lg", "状态事件列表" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示房间 {room_id} 的状态事件（事件类型、时间戳、内容、发送者）"
                }
            }
        }
    }
}

/// Media tab component (placeholder for Phase 3)
#[component]
fn MediaTab(room_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 媒体管理功能将在第三阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "🖼️" }
                p { class: "text-gray-500 text-lg", "媒体文件列表" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示房间 {room_id} 上传的媒体文件"
                }
            }
        }
    }
}

/// Forward extremities tab component (placeholder for Phase 3)
#[component]
fn ForwardExtremitiesTab(room_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 前沿终点管理功能将在第三阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "🔗" }
                p { class: "text-gray-500 text-lg", "前沿终点信息" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示房间 {room_id} 的前沿终点信息"
                }
            }
        }
    }
}

/// Format Unix timestamp to readable date string
fn format_timestamp(ts: u64) -> String {
    use chrono::{Utc, TimeZone};
    
    let dt = Utc.timestamp_opt(ts as i64, 0).single();
    match dt {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "无效时间".to_string(),
    }
}
