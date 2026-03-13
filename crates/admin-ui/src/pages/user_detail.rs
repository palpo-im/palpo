//! User detail page component with tabbed interface

use dioxus::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::app::Route;
use crate::models::user::User;
use crate::models::AuthState;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;
use crate::services::user_admin_api::UserAdminAPI;
use crate::utils::audit_logger::AuditLogger;
use crate::services::api_client::ApiClient;

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
    let user = use_signal(|| None::<User>);
    let loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let mut active_tab = use_signal(|| UserDetailTab::BasicInfo);
    let mut is_editing = use_signal(|| false);
    let mut edit_display_name = use_signal(String::new);
    let mut edit_avatar_url = use_signal(String::new);
    let mut edit_is_admin = use_signal(|| false);

    use_effect(move || {
        let user_id = user_id.clone();
        let mut loading = loading;
        let mut error = error;
        let mut user = user;
        let mut edit_display_name = edit_display_name;
        let mut edit_avatar_url = edit_avatar_url;
        let mut edit_is_admin = edit_is_admin;

        let auth_state = use_context::<Signal<AuthState>>();
        let admin_user = match &*auth_state.read() {
            AuthState::Authenticated(u) => u.username.clone(),
            _ => "admin".to_string(),
        };

        let audit_logger = AuditLogger::new(1000);
        let api_client = ApiClient::new("http://localhost:8081");
        let api = UserAdminAPI::new(audit_logger, api_client);

        spawn_local(async move {
            loading.set(true);
            error.set(None);

            match api.get_user(&user_id, &admin_user).await {
                Ok(Some(fetched_user)) => {
                    user.set(Some(fetched_user.clone()));
                    edit_display_name.set(fetched_user.display_name.clone().unwrap_or_default());
                    edit_avatar_url.set(fetched_user.avatar_url.clone().unwrap_or_default());
                    edit_is_admin.set(fetched_user.is_admin);
                }
                Ok(None) => {
                    error.set(Some(format!("用户 {} 不存在", user_id)));
                }
                Err(e) => {
                    error.set(Some(format!("获取用户信息失败: {}", e)));
                }
            }
            loading.set(false);
        });
    });

    use_effect(move || {
        if let Some(u) = user() {
            edit_display_name.set(u.display_name.clone().unwrap_or_default());
            edit_avatar_url.set(u.avatar_url.clone().unwrap_or_default());
            edit_is_admin.set(u.is_admin);
        }
    });

    rsx! {
        div { class: "space-y-6",
            div { class: "flex items-center gap-4",
                Link {
                    to: Route::Users {},
                    class: "inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50",
                    "← 返回用户列表"
                }
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "用户详情" }
                    if let Some(u) = user() {
                        p { class: "mt-1 text-sm text-gray-500", "{u.user_id}" }
                    }
                }
            }

            if loading() {
                div { class: "bg-white shadow rounded-lg p-12",
                    Spinner { size: "large".to_string(), message: Some("加载用户信息...".to_string()) }
                }
            } else if let Some(err) = error() {
                div { class: "bg-white shadow rounded-lg p-6",
                    ErrorMessage { message: err }
                }
            } else if let Some(u) = user() {
                div { class: "bg-white shadow rounded-lg",
                    div { class: "border-b border-gray-200",
                        nav { class: "flex -mb-px",
                            TabButton { label: "基本信息", icon: "👤", active: active_tab() == UserDetailTab::BasicInfo, onclick: move |_| active_tab.set(UserDetailTab::BasicInfo) },
                            TabButton { label: "权限管理", icon: "🔐", active: active_tab() == UserDetailTab::Permissions, onclick: move |_| active_tab.set(UserDetailTab::Permissions) },
                            TabButton { label: "设备", icon: "📱", active: active_tab() == UserDetailTab::Devices, onclick: move |_| active_tab.set(UserDetailTab::Devices) },
                            TabButton { label: "连接", icon: "🔌", active: active_tab() == UserDetailTab::Connections, onclick: move |_| active_tab.set(UserDetailTab::Connections) },
                            TabButton { label: "推送器", icon: "🔔", active: active_tab() == UserDetailTab::Pushers, onclick: move |_| active_tab.set(UserDetailTab::Pushers) },
                        }
                    }
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
                                    on_display_name_change: move |v: String| edit_display_name.set(v),
                                    on_avatar_url_change: move |v: String| edit_avatar_url.set(v),
                                    on_is_admin_change: move |v: bool| edit_is_admin.set(v),
                                    on_save: move |_| { is_editing.set(false); },
                                    on_cancel: move |_| {
                                        is_editing.set(false);
                                        if let Some(u) = user() {
                                            edit_display_name.set(u.display_name.clone().unwrap_or_default());
                                            edit_avatar_url.set(u.avatar_url.clone().unwrap_or_default());
                                            edit_is_admin.set(u.is_admin);
                                        }
                                    },
                                    on_lock: move |_| {},
                                    on_deactivate: move |_| {},
                                }
                            },
                            UserDetailTab::Permissions => rsx! { PermissionsTab { user: u.clone() } },
                            UserDetailTab::Devices => rsx! { DevicesTab { user_id: u.user_id.clone() } },
                            UserDetailTab::Connections => rsx! { ConnectionsTab { user_id: u.user_id.clone() } },
                            UserDetailTab::Pushers => rsx! { PushersTab { user_id: u.user_id.clone() } },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TabButton(label: String, icon: String, active: bool, onclick: EventHandler<()>) -> Element {
    let base = "group inline-flex items-center px-4 py-4 border-b-2 font-medium text-sm";
    let active_class = if active { "border-blue-500 text-blue-600" } else { "border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300" };
    rsx! {
        button { class: "{base} {active_class}", onclick: move |_| onclick.call(()), span { class: "mr-2", "{icon}" } "{label}" }
    }
}

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
            div { class: "flex items-start gap-6",
                div { class: "flex-shrink-0",
                    if let Some(url) = &user.avatar_url {
                        img { class: "h-24 w-24 rounded-full", src: "{url}", alt: "{user.username}" }
                    } else {
                        div { class: "h-24 w-24 rounded-full bg-gray-300 flex items-center justify-center text-gray-600 text-3xl font-semibold", "{user.username.chars().next().unwrap_or('U').to_uppercase()}" }
                    }
                }
                div { class: "flex-1",
                    if is_editing {
                        div { class: "space-y-4",
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "显示名" }
                                input { r#type: "text", class: "w-full px-3 py-2 border border-gray-300 rounded-md", value: "{edit_display_name}", oninput: move |evt| on_display_name_change.call(evt.value()) }
                            }
                            div {
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "头像 URL" }
                                input { r#type: "text", class: "w-full px-3 py-2 border border-gray-300 rounded-md", value: "{edit_avatar_url}", oninput: move |evt| on_avatar_url_change.call(evt.value()) }
                            }
                            div { class: "flex items-center",
                                input { r#type: "checkbox", id: "edit-is-admin", class: "h-4 w-4", checked: edit_is_admin, onchange: move |evt| on_is_admin_change.call(evt.checked()) }
                                label { r#for: "edit-is-admin", class: "ml-2 block text-sm text-gray-900", "管理员权限" }
                            }
                            div { class: "flex gap-2",
                                button { class: "px-4 py-2 bg-blue-600 text-white rounded-md", onclick: move |_| on_save.call(()), "💾 保存" },
                                button { class: "px-4 py-2 border border-gray-300 rounded-md", onclick: move |_| on_cancel.call(()), "取消" }
                            }
                        }
                    } else {
                        div { class: "space-y-3",
                            h3 { class: "text-2xl font-bold text-gray-900", "{user.display_name.as_ref().unwrap_or(&user.username)}" }
                            p { class: "text-sm text-gray-500", "@{user.username}" }
                            div { class: "flex gap-2",
                                if user.is_admin { span { class: "px-2.5 py-0.5 rounded-full text-xs font-medium bg-purple-100 text-purple-800", "🔐 管理员" } },
                                if user.is_deactivated { span { class: "px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800", "❌ 已停用" } } else { span { class: "px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800", "✓ 活跃" } }
                            }
                            p { class: "text-sm text-gray-600", "创建时间: {format_timestamp(user.creation_ts)}" }
                            if let Some(ls) = user.last_seen_ts { p { class: "text-sm text-gray-600", "最后活跃: {format_timestamp(ls)}" } }
                        }
                    }
                }
            }
            if !is_editing {
                div { class: "border-t border-gray-200 pt-6 flex flex-wrap gap-3",
                    button { class: "px-4 py-2 border border-gray-300 rounded-md text-gray-700", onclick: move |_| on_edit_toggle.call(()), "✏️ 编辑用户" },
                    button { class: "px-4 py-2 border border-gray-300 rounded-md text-gray-700", onclick: move |_| on_lock.call(()), if user.is_deactivated { "🔓 解锁用户" } else { "🔒 锁定用户" } },
                    button { class: "px-4 py-2 border border-red-300 rounded-md text-red-700", onclick: move |_| on_deactivate.call(()), if user.is_deactivated { "✓ 重新激活" } else { "❌ 停用用户" } }
                }
            }
        }
    }
}

#[component]
fn PermissionsTab(user: User) -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4", p { class: "text-sm text-blue-800", "ℹ️ 权限管理功能将在第二阶段实现" } }
            h3 { class: "text-lg font-medium text-gray-900 mb-4", "当前权限" }
            if user.permissions.is_empty() {
                p { class: "text-gray-500", "该用户暂无特殊权限" }
            } else {
                div { class: "space-y-2", for perm in &user.permissions {
                    div { span { class: "inline-flex items-center px-3 py-1 rounded-full text-sm font-medium bg-blue-100 text-blue-800", "{perm:?}" } }
                }}
            }
        }
    }
}

#[component]
fn DevicesTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            h3 { class: "text-lg font-medium text-gray-900", "设备管理" }
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4", p { class: "text-sm text-blue-800", "ℹ️ 设备管理功能将在第二阶段实现" } }
            div { class: "text-center py-12", div { class: "text-gray-400 text-5xl mb-4", "📱" }, p { class: "text-gray-500 text-lg", "设备列表" }, p { class: "text-gray-400 text-sm mt-2", "此处将显示用户 {user_id} 的所有设备" } }
        }
    }
}

#[component]
fn ConnectionsTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            h3 { class: "text-lg font-medium text-gray-900", "连接信息" }
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4", p { class: "text-sm text-blue-800", "ℹ️ 连接管理功能将在第二阶段实现" } }
            div { class: "text-center py-12", div { class: "text-gray-400 text-5xl mb-4", "🔌" }, p { class: "text-gray-500 text-lg", "连接信息" }, p { class: "text-gray-400 text-sm mt-2", "此处将显示用户 {user_id} 的连接信息" } }
        }
    }
}

#[component]
fn PushersTab(user_id: String) -> Element {
    rsx! {
        div { class: "space-y-6",
            h3 { class: "text-lg font-medium text-gray-900", "推送器配置" }
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4", p { class: "text-sm text-blue-800", "ℹ️ 推送器管理功能将在第二阶段实现" } }
            div { class: "text-center py-12", div { class: "text-gray-400 text-5xl mb-4", "🔔" }, p { class: "text-gray-500 text-lg", "推送器列表" }, p { class: "text-gray-400 text-sm mt-2", "此处将显示用户 {user_id} 的推送器配置" } }
        }
    }
}

fn format_timestamp(ts: u64) -> String {
    use chrono::{Utc, TimeZone};
    let dt = Utc.timestamp_opt(ts as i64, 0).single();
    match dt { Some(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(), None => "无效时间".to_string() }
}