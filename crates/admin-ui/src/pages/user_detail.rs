//! User detail page component with tabbed interface

use dioxus::prelude::*;
use dioxus_core::VNode;
use wasm_bindgen_futures::spawn_local;
use crate::app::Route;
use crate::models::user::User;
use crate::models::AuthState;
use crate::models::device::{DeviceInfo, DeviceListRequest};
use crate::models::session::{SessionInfo, SessionListRequest, WhoisInfo};
use crate::models::pusher::{PusherInfo, PusherListResponse};
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;
use crate::services::user_admin_api::UserAdminAPI;
use crate::utils::audit_logger::AuditLogger;
use crate::services::api_client::ApiClient;
use chrono::TimeZone;

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
    let auth_state = use_context::<Signal<AuthState>>();

    use_effect(move || {
        let user_id = user_id.clone();
        let mut loading = loading;
        let mut error = error;
        let mut user = user;
        let mut edit_display_name = edit_display_name;
        let mut edit_avatar_url = edit_avatar_url;
        let mut edit_is_admin = edit_is_admin;

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
        div { class: "p-4 sm:p-6 space-y-6",
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
        div { class: "p-4 sm:p-6 space-y-6",
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
        div { class: "p-4 sm:p-6 space-y-6",
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
    let devices = use_signal(|| Vec::<DeviceInfo>::new());
    let loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let selected_devices = use_signal(|| Vec::<String>::new());
    let mut show_delete_confirm = use_signal(|| None::<String>); // device_id to delete
    let auth_state = use_context::<Signal<AuthState>>();

    // Clone user_id for delete_device closure
    let user_id_for_delete = user_id.clone();

    use_effect(move || {
        let user_id = user_id.clone();
        let mut loading = loading;
        let mut error = error;
        let mut devices = devices;

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

            match api.get_user_devices(&user_id, DeviceListRequest::default(), &admin_user).await {
                Ok(response) => {
                    if response.success {
                        devices.set(response.devices);
                    } else {
                        error.set(response.error.or(Some("获取设备列表失败".to_string())));
                    }
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }
            loading.set(false);
        });
    });

    let delete_device = move |device_id: String| {
        let user_id = user_id_for_delete.clone();
        let mut error = error;
        let mut devices = devices;
        let mut show_delete_confirm = show_delete_confirm;

        let admin_user = match &*auth_state.read() {
            AuthState::Authenticated(u) => u.username.clone(),
            _ => "admin".to_string(),
        };

        let audit_logger = AuditLogger::new(1000);
        let api_client = ApiClient::new("http://localhost:8081");
        let api = UserAdminAPI::new(audit_logger, api_client);

        spawn_local(async move {
            match api.delete_device(&user_id, &device_id, &admin_user).await {
                Ok(response) => {
                    if response.success {
                        // Remove from local list
                        devices.set(devices().into_iter().filter(|d| d.device_id != device_id).collect());
                    } else {
                        error.set(response.error.or(Some("删除设备失败".to_string())));
                    }
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }
            show_delete_confirm.set(None);
        });
    };

    rsx! {
        div { class: "p-4 sm:p-6 space-y-6",
            div { class: "flex items-center justify-between",
                h3 { class: "text-lg font-medium text-gray-900", "设备管理" }
                if !devices().is_empty() {
                    div { class: "text-sm text-gray-500", "共 {devices().len()} 个设备" }
                }
            }

            if let Some(err) = error() {
                div { class: "p-4 bg-red-50 border border-red-200 rounded-md",
                    p { class: "text-sm text-red-600", "{err}" }
                }
            }

            if loading() {
                div { class: "p-12 text-center",
                    Spinner { size: "large".to_string(), message: Some("加载设备列表...".to_string()) }
                }
            } else if devices().is_empty() {
                div { class: "text-center py-12",
                    div { class: "text-gray-400 text-5xl mb-4", "📱" }
                    p { class: "text-gray-500 text-lg", "暂无设备" }
                    p { class: "text-gray-400 text-sm mt-2", "该用户还没有关联任何设备" }
                }
            } else {
                div { class: "overflow-x-auto",
                    table { class: "min-w-full divide-y divide-gray-200",
                        thead { class: "bg-gray-50",
                            tr {
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "设备" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "IP 地址" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "最后活跃" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "状态" }
                                th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                            }
                        }
                        tbody { class: "bg-white divide-y divide-gray-200",
                            for device in devices() {
                                tr { class: "hover:bg-gray-50",
                                    td { class: "px-6 py-4 whitespace-nowrap",
                                        div { class: "flex items-center",
                                            span { class: "text-2xl mr-3", "{device.device_icon()}" }
                                            div {
                                                div { class: "text-sm font-medium text-gray-900",
                                                    "{device.display_name.as_ref().unwrap_or(&device.device_id)}"
                                                }
                                                div { class: "text-xs text-gray-500 font-mono", "{device.device_id}" }
                                            }
                                        }
                                    }
                                    td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                                        "{device.last_seen_ip.as_ref().unwrap_or(&String::from(\"-\"))}"
                                    }
                                    td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                                        "{device.last_seen_readable()}"
                                    }
                                    td { class: "px-6 py-4 whitespace-nowrap",
                                        if device.is_suspended {
                                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-yellow-100 text-yellow-800", "已暂停" }
                                        } else if device.is_active() {
                                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800", "活跃" }
                                        } else {
                                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800", "离线" }
                                        }
                                    }
                                    td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                                        button {
                                            class: "text-red-600 hover:text-red-900",
                                            onclick: move |_| show_delete_confirm.set(Some(device.device_id.clone())),
                                            "删除"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Delete confirmation dialog
            if let Some(device_id) = show_delete_confirm() {
                crate::components::dialogs::ConfirmationDialog {
                    dialog_type: crate::components::dialogs::DialogType::DeleteDevice,
                    title: "确认删除设备".to_string(),
                    message: "确定要删除此设备吗？该用户将需要重新登录才能使用此设备。".to_string(),
                    target: device_id.clone(),
                    visible: true,
                    loading: loading(),
                    on_confirm: move |_| delete_device(device_id.clone()),
                    on_cancel: move |_| show_delete_confirm.set(None),
                }
            }
        }
    }
}

#[component]
fn ConnectionsTab(user_id: String) -> Element {
    let sessions = use_signal(|| Vec::<SessionInfo>::new());
    let whois = use_signal(|| None::<WhoisInfo>);
    let loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let active_sessions = use_signal(|| 0u32);
    let current_user_id = use_signal(|| user_id.clone());
    let auth_state = use_context::<Signal<AuthState>>();

    use_effect(move || {
        let user_id = user_id.clone();
        let mut loading = loading;
        let mut error = error;
        let mut sessions = sessions;
        let mut whois = whois;
        let mut active_sessions = active_sessions;

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

            match api.get_user_sessions(&user_id, SessionListRequest::default(), &admin_user).await {
                Ok(response) => {
                    if response.success {
                        sessions.set(response.sessions.clone());
                        active_sessions.set(response.active_count);
                    } else {
                        error.set(response.error.or(Some("获取会话列表失败".to_string())));
                    }
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }

            // Fetch whois info
            match api.get_whois(&user_id, &admin_user).await {
                Ok(response) => {
                    whois.set(Some(response));
                }
                Err(_) => {
                    // Whois is optional, don't show error
                }
            }

            loading.set(false);
        });
    });

    rsx! {
        div { class: "p-4 sm:p-6 space-y-6",
            div { class: "flex items-center justify-between",
                h3 { class: "text-lg font-medium text-gray-900", "连接信息" }
                if !sessions().is_empty() {
                    div { class: "text-sm text-gray-500", "共 {sessions().len()} 个会话，{active_sessions()} 个活跃" }
                }
            }

            if let Some(err) = error() {
                div { class: "p-4 bg-red-50 border border-red-200 rounded-md",
                    p { class: "text-sm text-red-600", "{err}" }
                }
            }

            if loading() {
                div { class: "p-12 text-center",
                    Spinner { size: "large".to_string(), message: Some("加载连接信息...".to_string()) }
                }
            } else if sessions().is_empty() {
                div { class: "text-center py-12",
                    div { class: "text-gray-400 text-5xl mb-4", "🔌" }
                    p { class: "text-gray-500 text-lg", "暂无连接" }
                    p { class: "text-gray-400 text-sm mt-2", "该用户当前没有活跃的连接" }
                }
            } else {
                div { class: "grid gap-4",
                    for session in sessions() {
                        div { class: "bg-white border border-gray-200 rounded-lg p-4 hover:shadow-sm",
                            div { class: "flex items-start justify-between",
                                div { class: "flex items-start",
                                    span { class: "text-3xl mr-4", "🖥️" }
                                    div {
                                        div { class: "flex items-center gap-2",
                                            h4 { class: "text-sm font-medium text-gray-900",
                                                "{display_device_name(session.device_display_name.clone())}"
                                            }
                                            if session.is_active {
                                                span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                                                    "活跃"
                                                }
                                            } else {
                                                span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                                                    "离线"
                                                }
                                            }
                                        }
                                        p { class: "text-xs text-gray-500 mt-1",
                                            "IP: {display_ip_address(session.ip_address.clone())} • {display_user_agent(session.user_agent.clone())}"
                                        }
                                        p { class: "text-xs text-gray-400 mt-1",
                                            "登录时间: {format_timestamp(session.login_ts)} • 最后活动: {format_timestamp(session.last_activity_ts)}"
                                        }
                                    }
                                }
                                button {
                                    class: "text-red-600 hover:text-red-900 text-sm",
                                    disabled: !session.is_active,
                                    onclick: move |_| {
                                        let session_id = session.session_id.clone();
                                        let user_id = current_user_id();
                                        let mut sessions = sessions;
                                        let mut error = error;
                                        
                                        let admin_user = match &*auth_state.read() {
                                            AuthState::Authenticated(u) => u.username.clone(),
                                            _ => "admin".to_string(),
                                        };
                                        
                                        let audit_logger = AuditLogger::new(1000);
                                        let api_client = ApiClient::new("http://localhost:8081");
                                        let api = UserAdminAPI::new(audit_logger, api_client);
                                        
                                        spawn_local(async move {
                                            match api.terminate_session(&user_id, &session_id, &admin_user).await {
                                                Ok(response) => {
                                                    if response.success {
                                                        sessions.set(sessions().into_iter().filter(|s| s.session_id != session_id).collect());
                                                    } else {
                                                        error.set(response.error.or(Some("终止会话失败".to_string())));
                                                    }
                                                }
                                                Err(e) => {
                                                    error.set(Some(e.to_string()));
                                                }
                                            }
                                        });
                                    },
                                    "终止"
                                }
                            }
                            div { class: "mt-3 pt-3 border-t border-gray-100 text-xs text-gray-500",
                                span { "会话ID: {session.session_id}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PushersTab(user_id: String) -> Element {
    let pushers = use_signal(|| Vec::<PusherInfo>::new());
    let loading = use_signal(|| true);
    let error = use_signal(|| None::<String>);
    let current_user_id = use_signal(|| user_id.clone());
    let auth_state = use_context::<Signal<AuthState>>();

    use_effect(move || {
        let user_id = user_id.clone();
        let mut loading = loading;
        let mut error = error;
        let mut pushers = pushers;

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

            match api.get_user_pushers(&user_id, &admin_user).await {
                Ok(response) => {
                    if response.success {
                        pushers.set(response.pushers);
                    } else {
                        error.set(response.error.or(Some("获取推送器列表失败".to_string())));
                    }
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                }
            }
            loading.set(false);
        });
    });

    rsx! {
        div { class: "p-4 sm:p-6 space-y-6",
            div { class: "flex items-center justify-between",
                h3 { class: "text-lg font-medium text-gray-900", "推送器配置" }
                if !pushers().is_empty() {
                    div { class: "text-sm text-gray-500", "共 {pushers().len()} 个推送器" }
                }
            }

            if let Some(err) = error() {
                div { class: "p-4 bg-red-50 border border-red-200 rounded-md",
                    p { class: "text-sm text-red-600", "{err}" }
                }
            }

            if loading() {
                div { class: "p-12 text-center",
                    Spinner { size: "large".to_string(), message: Some("加载推送器列表...".to_string()) }
                }
            } else if pushers().is_empty() {
                div { class: "text-center py-12",
                    div { class: "text-gray-400 text-5xl mb-4", "🔔" }
                    p { class: "text-gray-500 text-lg", "暂无推送器" }
                    p { class: "text-gray-400 text-sm mt-2", "该用户还没有配置任何推送器" }
                }
            } else {
                div { class: "grid gap-4",
                    for pusher in pushers() {
                        div { class: "bg-white border border-gray-200 rounded-lg p-4 hover:shadow-sm",
                            div { class: "flex items-start justify-between",
                                div { class: "flex items-start",
                                    span { class: "text-3xl mr-4", "{pusher.icon()}" }
                                    div {
                                        div { class: "flex items-center gap-2",
                                            h4 { class: "text-sm font-medium text-gray-900",
                                                "{pusher.app_display_name}"
                                            }
                                            if pusher.state == crate::models::pusher::PusherState::Active {
                                                span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                                                    "{pusher.state_display()}"
                                                }
                                            } else {
                                                span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                                                    "{pusher.state_display()}"
                                                }
                                            }
                                        }
                                        p { class: "text-xs text-gray-500 mt-1",
                                            "{pusher.kind_display()} • {pusher.lang}"
                                        }
                                        if let Some(url_val) = &pusher.data.url {
                                            p { class: "text-xs text-gray-400 font-mono mt-1 break-all", "{url_val}" }
                                        }
                                    }
                                }
                                button {
                                    class: "text-red-600 hover:text-red-900 text-sm",
                                    onclick: move |_| {
                                        let pusher_id = pusher.pusher_id.clone();
                                        let user_id = current_user_id();
                                        let mut pushers = pushers;
                                        let mut error = error;
                                        
                                        let admin_user = match &*auth_state.read() {
                                            AuthState::Authenticated(u) => u.username.clone(),
                                            _ => "admin".to_string(),
                                        };
                                        
                                        let audit_logger = AuditLogger::new(1000);
                                        let api_client = ApiClient::new("http://localhost:8081");
                                        let api = UserAdminAPI::new(audit_logger, api_client);
                                        
                                        spawn_local(async move {
                                            match api.delete_pusher(&user_id, &pusher_id, &admin_user).await {
                                                Ok(response) => {
                                                    if response.success {
                                                        pushers.set(pushers().into_iter().filter(|p| p.pusher_id != pusher_id).collect());
                                                    } else {
                                                        error.set(response.error.or(Some("删除推送器失败".to_string())));
                                                    }
                                                }
                                                Err(e) => {
                                                    error.set(Some(e.to_string()));
                                                }
                                            }
                                        });
                                    },
                                    "删除"
                                }
                            }
                            div { class: "mt-3 pt-3 border-t border-gray-100 flex items-center justify-between text-xs text-gray-500",
                                div { class: "flex items-center gap-4",
                                    span { "ID: {pusher.pusher_id}" }
                                    span { "{format_pusher_last_active(pusher.last_active_ts)}" }
                                }
                                if let Some(profile) = &pusher.profile_tag {
                                    span { class: "font-mono", "Profile: {profile}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_timestamp(ts: u64) -> String {
    use chrono::{Utc, TimeZone};
    let dt = Utc.timestamp_opt(ts as i64, 0).single();
    match dt { Some(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(), None => "无效时间".to_string() }
}

fn format_pusher_last_active(ts: Option<u64>) -> String {
    match ts {
        Some(timestamp) => {
            let dt = chrono::Utc.timestamp_opt(timestamp as i64, 0).single();
            match dt {
                Some(d) => format!("最后活跃: {}", d.format("%Y-%m-%d %H:%M").to_string()),
                None => "最后活跃: 未知".to_string()
            }
        }
        None => "最后活跃: 从未".to_string()
    }
}

fn display_device_name(device_display_name: Option<String>) -> String {
    device_display_name.unwrap_or("未知设备".to_string())
}

fn display_ip_address(ip_address: Option<String>) -> String {
    ip_address.unwrap_or("未知".to_string())
}

fn display_user_agent(user_agent: Option<String>) -> String {
    user_agent.unwrap_or("未知浏览器".to_string())
}
