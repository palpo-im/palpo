//! User detail page component with tabbed interface

use dioxus::prelude::*;
use crate::app::Route;
use crate::models::user::User;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Tab types for user detail page
#[derive(Clone, Copy, Debug, PartialEq)]
enum UserDetailTab {
    BasicInfo,
    Permissions,
    Devices,
    Connections,
    Pushers,
}

/// User detail page component with tabbed interface
#[component]
pub fn UserDetail(user_id: String) -> Element {
    // State management
    let mut user = use_signal(|| None::<User>);
    let mut loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let mut active_tab = use_signal(|| UserDetailTab::BasicInfo);
    let mut is_editing = use_signal(|| false);
    
    // Edit form state
    let mut edit_display_name = use_signal(String::new);
    let mut edit_avatar_url = use_signal(String::new);
    let mut edit_is_admin = use_signal(|| false);

    // Load user data effect (placeholder - will be implemented in Phase 2)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 2
        // For now, create a mock user for UI testing
        let mock_user = User {
            user_id: user_id.clone(),
            username: user_id.split(':').next().unwrap_or("user").trim_start_matches('@').to_string(),
            display_name: Some("Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_deactivated: false,
            creation_ts: 1640000000,
            last_seen_ts: Some(1640100000),
            permissions: vec![],
        };
        user.set(Some(mock_user));
        loading.set(false);
    });

    // Initialize edit form when user data loads
    use_effect(move || {
        if let Some(u) = user() {
            edit_display_name.set(u.display_name.clone().unwrap_or_default());
            edit_avatar_url.set(u.avatar_url.clone().unwrap_or_default());
            edit_is_admin.set(u.is_admin);
        }
    });

    rsx! {
        div { class: "space-y-6",
            // Header with back button
            div { class: "flex items-center gap-4",
                Link {
                    to: Route::Users {},
                    class: "inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                    "← 返回用户列表"
                }
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "用户详情" }
                    if let Some(u) = user() {
                        p { class: "mt-1 text-sm text-gray-500", "{u.user_id}" }
                    }
                }
            }

            // Loading state
            if loading() {
                div { class: "bg-white shadow rounded-lg p-12",
                    Spinner { size: "large".to_string(), message: Some("加载用户信息...".to_string()) }
                }
            } else if let Some(err) = error() {
                div { class: "bg-white shadow rounded-lg p-6",
                    ErrorMessage { message: err }
                }
            } else if let Some(u) = user() {
                // Tab navigation
                div { class: "bg-white shadow rounded-lg",
                    div { class: "border-b border-gray-200",
                        nav { class: "flex -mb-px",
                            TabButton {
                                label: "基本信息",
                                icon: "👤",
                                active: active_tab() == UserDetailTab::BasicInfo,
                                onclick: move |_| active_tab.set(UserDetailTab::BasicInfo)
                            }
                            TabButton {
                                label: "权限管理",
                                icon: "🔐",
                                active: active_tab() == UserDetailTab::Permissions,
                                onclick: move |_| active_tab.set(UserDetailTab::Permissions)
                            }
                            TabButton {
                                label: "设备",
                                icon: "📱",
                                active: active_tab() == UserDetailTab::Devices,
                                onclick: move |_| active_tab.set(UserDetailTab::Devices)
                            }
                            TabButton {
                                label: "连接",
                                icon: "🔌",
                                active: active_tab() == UserDetailTab::Connections,
                                onclick: move |_| active_tab.set(UserDetailTab::Connections)
                            }
                            TabButton {
                                label: "推送器",
                                icon: "🔔",
                                active: active_tab() == UserDetailTab::Pushers,
                                onclick: move |_| active_tab.set(UserDetailTab::Pushers)
                            }
                        }
                    }

                    // Tab content
                    div { class: "p-6",
                        match active_tab() {
                            UserDetailTab::BasicInfo => rsx! {
                                BasicInfoTab {
                                    user: u.clone(),
                                    is_editing: is_editing(),
                                    edit_display_name: edit_display_name(),
                                    edit_avatar_url: edit_avatar_url(),
                                    edit_is_admin: edit_is_admin(),
                                    on_edit_toggle: move |_| is_editing.set(!is_editing()),
                                    on_display_name_change: move |value: String| edit_display_name.set(value),
                                    on_avatar_url_change: move |value: String| edit_avatar_url.set(value),
                                    on_is_admin_change: move |value: bool| edit_is_admin.set(value),
                                    on_save: move |_| {
                                        // TODO: Implement save in Phase 2
                                        is_editing.set(false);
                                    },
                                    on_cancel: move |_| {
                                        is_editing.set(false);
                                        // Reset form
                                        if let Some(u) = user() {
                                            edit_display_name.set(u.display_name.clone().unwrap_or_default());
                                            edit_avatar_url.set(u.avatar_url.clone().unwrap_or_default());
                                            edit_is_admin.set(u.is_admin);
                                        }
                                    },
                                    on_lock: move |_| {
                                        // TODO: Implement lock in Phase 2
                                    },
                                    on_deactivate: move |_| {
                                        // TODO: Implement deactivate in Phase 2
                                    }
                                }
                            },
                            UserDetailTab::Permissions => rsx! {
                                PermissionsTab {
                                    user: u.clone()
                                }
                            },
                            UserDetailTab::Devices => rsx! {
                                DevicesTab {
                                    user_id: u.user_id.clone()
                                }
                            },
                            UserDetailTab::Connections => rsx! {
                                ConnectionsTab {
                                    user_id: u.user_id.clone()
                                }
                            },
                            UserDetailTab::Pushers => rsx! {
                                PushersTab {
                                    user_id: u.user_id.clone()
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
    user: User,
    is_editing: bool,
    edit_display_name: String,
    edit_avatar_url: String,
    edit_is_admin: bool,
    on_edit_toggle: EventHandler<()>,
    on_display_name_change: EventHandler<String>,
    on_avatar_url_change: EventHandler<String>,
    on_is_admin_change: EventHandler<bool>,
    on_save: EventHandler<()>,
    on_cancel: EventHandler<()>,
    on_lock: EventHandler<()>,
    on_deactivate: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            // User avatar and basic info
            div { class: "flex items-start gap-6",
                // Avatar
                div { class: "flex-shrink-0",
                    if let Some(avatar_url) = &user.avatar_url {
                        img {
                            class: "h-24 w-24 rounded-full",
                            src: "{avatar_url}",
                            alt: "{user.username}"
                        }
                    } else {
                        div { class: "h-24 w-24 rounded-full bg-gray-300 flex items-center justify-center text-gray-600 text-3xl font-semibold",
                            "{user.username.chars().next().unwrap_or('U').to_uppercase()}"
                        }
                    }
                }

                // User info
                div { class: "flex-1",
                    if is_editing {
                        // Edit form
                        div { class: "space-y-4",
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "显示名" }
                                input {
                                    r#type: "text",
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                    value: "{edit_display_name}",
                                    oninput: move |evt| on_display_name_change.call(evt.value())
                                }
                            }
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "头像 URL" }
                                input {
                                    r#type: "text",
                                    class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                    value: "{edit_avatar_url}",
                                    placeholder: "https://example.com/avatar.jpg",
                                    oninput: move |evt| on_avatar_url_change.call(evt.value())
                                }
                            }
                            div { class: "flex items-center",
                                input {
                                    r#type: "checkbox",
                                    id: "edit-is-admin",
                                    class: "h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded",
                                    checked: edit_is_admin,
                                    onchange: move |evt| on_is_admin_change.call(evt.checked())
                                }
                                label {
                                    r#for: "edit-is-admin",
                                    class: "ml-2 block text-sm text-gray-900",
                                    "管理员权限"
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
                                    "{user.display_name.as_ref().unwrap_or(&user.username)}"
                                }
                                p { class: "text-sm text-gray-500", "@{user.username}" }
                            }
                            div { class: "flex gap-2",
                                if user.is_admin {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-purple-100 text-purple-800",
                                        "🔐 管理员"
                                    }
                                }
                                if user.is_deactivated {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                                        "❌ 已停用"
                                    }
                                } else {
                                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                                        "✓ 活跃"
                                    }
                                }
                            }
                            div { class: "text-sm text-gray-600",
                                p { "创建时间: {format_timestamp(user.creation_ts)}" }
                                if let Some(last_seen) = user.last_seen_ts {
                                    p { "最后活跃: {format_timestamp(last_seen)}" }
                                }
                            }
                        }
                    }
                }
            }

            // Action buttons (only in display mode)
            if !is_editing {
                div { class: "border-t border-gray-200 pt-6",
                    div { class: "flex flex-wrap gap-3",
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                            onclick: move |_| on_edit_toggle.call(()),
                            "✏️ 编辑用户"
                        }
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                            onclick: move |_| on_lock.call(()),
                            if user.is_deactivated {
                                "🔓 解锁用户"
                            } else {
                                "🔒 锁定用户"
                            }
                        }
                        button {
                            class: "inline-flex items-center px-4 py-2 border border-red-300 text-sm font-medium rounded-md text-red-700 bg-white hover:bg-red-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-red-500",
                            onclick: move |_| on_deactivate.call(()),
                            if user.is_deactivated {
                                "✓ 重新激活"
                            } else {
                                "❌ 停用用户"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Permissions tab component (placeholder for Phase 2)
#[component]
fn PermissionsTab(user: User) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 权限管理功能将在第二阶段实现"
                }
            }
            
            div {
                h3 { class: "text-lg font-medium text-gray-900 mb-4", "当前权限" }
                if user.permissions.is_empty() {
                    p { class: "text-gray-500", "该用户暂无特殊权限" }
                } else {
                    div { class: "space-y-2",
                        for permission in &user.permissions {
                            div { class: "flex items-center gap-2",
                                span { class: "inline-flex items-center px-3 py-1 rounded-full text-sm font-medium bg-blue-100 text-blue-800",
                                    "{permission:?}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Devices tab component (placeholder for Phase 2)
#[component]
fn DevicesTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 设备管理功能将在第二阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "📱" }
                p { class: "text-gray-500 text-lg", "设备列表" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示用户 {user_id} 的所有设备"
                }
            }
        }
    }
}

/// Connections tab component (placeholder for Phase 2)
#[component]
fn ConnectionsTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 连接管理功能将在第二阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "🔌" }
                p { class: "text-gray-500 text-lg", "连接信息" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示用户 {user_id} 的连接信息（IP地址、最后活跃时间、用户代理）"
                }
            }
        }
    }
}

/// Pushers tab component (placeholder for Phase 2)
#[component]
fn PushersTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                p { class: "text-sm text-blue-800",
                    "ℹ️ 推送器管理功能将在第二阶段实现"
                }
            }
            
            div { class: "text-center py-12",
                div { class: "text-gray-400 text-5xl mb-4", "🔔" }
                p { class: "text-gray-500 text-lg", "推送器列表" }
                p { class: "text-gray-400 text-sm mt-2",
                    "此处将显示用户 {user_id} 的推送器配置"
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
