//! Matrix Admin Creation Page Component
//!
//! This module provides a web interface for creating Matrix admin users.
//! It requires the Palpo server to be running and provides a form for
//! creating new Matrix admin accounts with admin privileges.
//!
//! # Features
//!
//! - Server status check before showing form
//! - Username, password, and display name fields
//! - Real-time password policy validation
//! - Display created credentials with copy buttons
//! - Link to server control if server not running
//!
//! # Requirements
//!
//! Implements requirements:
//! - 7.1: Verify Palpo server is running before creation
//! - 7.2: Return clear error if server not running
//! - 7.3: Use Matrix Admin API endpoint
//! - 7.4: Set admin field to 1 (true)
//! - 7.5: Validate password policy
//! - 7.6: Return created username and password
//! - 7.7: Verify admin status after creation

use dioxus::prelude::*;
use crate::services::api_client::get_api_client;
use serde::{Deserialize, Serialize};

/// Password policy validation result
#[derive(Clone, Debug, PartialEq, Default)]
struct PasswordValidation {
    min_length: bool,
    has_uppercase: bool,
    has_lowercase: bool,
    has_digit: bool,
    has_special: bool,
}

impl PasswordValidation {
    fn validate(password: &str) -> Self {
        Self {
            min_length: password.len() >= 12,
            has_uppercase: password.chars().any(|c| c.is_uppercase()),
            has_lowercase: password.chars().any(|c| c.is_lowercase()),
            has_digit: password.chars().any(|c| c.is_ascii_digit()),
            has_special: password.chars().any(|c| !c.is_alphanumeric()),
        }
    }

    fn is_valid(&self) -> bool {
        self.min_length
            && self.has_uppercase
            && self.has_lowercase
            && self.has_digit
            && self.has_special
    }
}

/// Server status enum matching backend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerStatus {
    NotStarted,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error,
}

/// Server status information from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusInfo {
    pub status: ServerStatus,
    pub pid: Option<u32>,
    pub uptime_seconds: Option<i64>,
}

/// Request body for creating Matrix admin
#[derive(Debug, Serialize)]
struct CreateMatrixAdminRequest {
    username: String,
    password: String,
    displayname: Option<String>,
}

/// Response for Matrix admin creation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMatrixAdminResponse {
    pub user_id: String,
    pub username: String,
    pub password: String,
    pub message: String,
}

/// Main Matrix admin creation page component
///
/// This component provides a form for creating Matrix admin users.
/// It first checks if the server is running, and if not, shows
/// an error with a link to the server control page.
///
/// # State Management
///
/// - `is_loading`: Whether an operation is in progress
/// - `error_message`: Error message to display
/// - `success_message`: Success message to display
/// - `server_status`: Current server status
/// - `username`: Username input value
/// - `password`: Password input value
/// - `confirm_password`: Confirm password input value
/// - `displayname`: Display name input value
/// - `password_validation`: Current password validation state
/// - `created_credentials`: Created credentials to display
#[component]
pub fn MatrixAdminCreatePage() -> Element {
    let mut is_loading = use_signal(|| false);
    let mut error_message = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);
    let mut server_status = use_signal(|| None::<ServerStatusInfo>);
    let mut username = use_signal(|| "".to_string());
    let mut password = use_signal(|| "".to_string());
    let mut confirm_password = use_signal(|| "".to_string());
    let mut displayname = use_signal(|| "".to_string());
    let mut password_validation = use_signal(|| PasswordValidation::default());
    let mut created_credentials = use_signal(|| None::<CreateMatrixAdminResponse>);

    // Check server status on mount
    let check_status = use_callback(move |_| async move {
        match get_api_client() {
            Ok(client) => {
                match client.get_json::<ServerStatusInfo>("/api/v1/admin/server/status").await {
                    Ok(status) => {
                        server_status.set(Some(status));
                    }
                    Err(_) => {
                        server_status.set(None);
                    }
                }
            }
            Err(_) => {
                server_status.set(None);
            }
        }
    });

    use_effect(move || {
        check_status.call(());
    });

    // Validate password on change
    use_effect(move || {
        let validation = PasswordValidation::validate(&password());
        password_validation.set(validation);
    });

    // Handle form submission
    let handle_submit = move |_| {
        spawn(async move {
            // Validate passwords match
            if password() != confirm_password() {
                error_message.set(Some("两次输入的密码不匹配".to_string()));
                return;
            }

            // Validate password
            if !password_validation().is_valid() {
                error_message.set(Some("密码不符合策略要求".to_string()));
                return;
            }

            // Validate username
            if username().trim().is_empty() {
                error_message.set(Some("用户名不能为空".to_string()));
                return;
            }

            is_loading.set(true);
            error_message.set(None);
            success_message.set(None);

            let request = CreateMatrixAdminRequest {
                username: username().trim().to_string(),
                password: password().to_string(),
                displayname: if displayname().trim().is_empty() {
                    None
                } else {
                    Some(displayname().trim().to_string())
                },
            };

            match get_api_client() {
                Ok(client) => {
                    match client.post_json_response::<CreateMatrixAdminRequest, CreateMatrixAdminResponse>(
                        "/api/v1/admin/matrix-admin/create",
                        &request,
                    ).await {
                        Ok(resp) => {
                            success_message.set(Some(resp.message.clone()));
                            created_credentials.set(Some(resp));
                            // Clear sensitive data
                            password.set("".to_string());
                            confirm_password.set("".to_string());
                        }
                        Err(e) => {
                            error_message.set(Some(format!("创建失败: {}", e)));
                        }
                    }
                }
                Err(e) => {
                    error_message.set(Some(format!("API客户端错误: {}", e)));
                }
            }

            is_loading.set(false);
        });
    };

    // Copy to clipboard handler
    let copy_to_clipboard = move |text: String| {
        let text = text.clone();
        spawn(async move {
            if let Some(window) = web_sys::window() {
                let navigator = window.navigator();
                // Clipboard API is only available in secure contexts
                // Safely check if clipboard is available before using it
                let has_clipboard = js_sys::Reflect::has(&navigator, &"clipboard".into())
                    .unwrap_or(false);
                
                if has_clipboard {
                    let clipboard = navigator.clipboard();
                    let _ = clipboard.write_text(&text);
                }
            }
        });
    };

    rsx! {
        div { class: "space-y-6",
            // Header
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "创建 Matrix 管理员"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "创建具有 Matrix 管理员权限的新用户"
                    }
                }
            }

            // Success/Error messages
            if let Some(success) = success_message() {
                div { class: "bg-white shadow rounded-lg",
                    div { class: "px-4 py-5 sm:p-6",
                        div { class: "rounded-md bg-green-50 p-4",
                            div { class: "flex",
                                div { class: "flex-shrink-0",
                                    svg { class: "h-5 w-5 text-green-400", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                        path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z", clip_rule: "evenodd" }
                                    }
                                }
                                div { class: "ml-3",
                                    p { class: "text-sm font-medium text-green-800", "{success}" }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(error) = error_message() {
                div { class: "bg-white shadow rounded-lg",
                    div { class: "px-4 py-5 sm:p-6",
                        div { class: "rounded-md bg-red-50 p-4",
                            div { class: "flex",
                                div { class: "flex-shrink-0",
                                    svg { class: "h-5 w-5 text-red-400", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                        path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                    }
                                }
                                div { class: "ml-3",
                                    p { class: "text-sm font-medium text-red-800", "{error}" }
                                }
                            }
                        }
                    }
                }
            }

            // Server status check
            if let Some(status) = server_status() {
                if status.status != ServerStatus::Running {
                    div { class: "bg-white shadow rounded-lg",
                        div { class: "px-4 py-5 sm:p-6",
                            div { class: "rounded-md bg-yellow-50 p-4",
                                div { class: "flex",
                                    div { class: "flex-shrink-0",
                                        svg { class: "h-5 w-5 text-yellow-400", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                            path { fill_rule: "evenodd", d: "M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z", clip_rule: "evenodd" }
                                        }
                                    }
                                    div { class: "ml-3",
                                        h3 { class: "text-sm font-medium text-yellow-800",
                                            "服务器未运行"
                                        }
                                        div { class: "mt-2 text-sm text-yellow-700",
                                            p { "Matrix 管理员创建需要 Palpo 服务器运行。请先启动服务器。" }
                                        }
                                        div { class: "mt-4",
                                            a {
                                                href: "/server-control",
                                                class: "inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md text-yellow-800 bg-yellow-100 hover:bg-yellow-200",
                                                "前往服务器控制"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Creation form
                    div { class: "bg-white shadow rounded-lg",
                        div { class: "px-4 py-5 sm:p-6",
                            form {
                                onsubmit: move |evt| {
                                    evt.prevent_default();
                                    handle_submit(());
                                },

                                // Username
                                div { class: "mb-4",
                                    label { class: "block text-sm font-medium text-gray-700",
                                        "用户名"
                                    }
                                    div { class: "mt-1",
                                        input {
                                            r#type: "text",
                                            value: "{username}",
                                            oninput: move |e| username.set(e.value().clone()),
                                            class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                            placeholder: "输入用户名",
                                        }
                                    }
                                    p { class: "mt-1 text-xs text-gray-500",
                                        "用户名不能包含空格或特殊字符"
                                    }
                                }

                                // Display name
                                div { class: "mb-4",
                                    label { class: "block text-sm font-medium text-gray-700",
                                        "显示名称 (可选)"
                                    }
                                    div { class: "mt-1",
                                        input {
                                            r#type: "text",
                                            value: "{displayname}",
                                            oninput: move |e| displayname.set(e.value().clone()),
                                            class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                            placeholder: "输入显示名称",
                                        }
                                    }
                                }

                                // Password
                                div { class: "mb-4",
                                    label { class: "block text-sm font-medium text-gray-700",
                                        "密码"
                                    }
                                    div { class: "mt-1",
                                        input {
                                            r#type: "password",
                                            value: "{password}",
                                            oninput: move |e| password.set(e.value().clone()),
                                            class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                            placeholder: "输入密码",
                                        }
                                    }
                                    // Password requirements
                                    div { class: "mt-2 space-y-1",
                                        div { class: "flex items-center text-xs",
                                            if password_validation().min_length {
                                                svg { class: "h-4 w-4 text-green-500 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z", clip_rule: "evenodd" }
                                                }
                                            } else {
                                                svg { class: "h-4 w-4 text-gray-300 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                                }
                                            }
                                            span { class: if password_validation().min_length { "text-green-700" } else { "text-gray-500" }, "至少12个字符" }
                                        }
                                        div { class: "flex items-center text-xs",
                                            if password_validation().has_uppercase {
                                                svg { class: "h-4 w-4 text-green-500 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z", clip_rule: "evenodd" }
                                                }
                                            } else {
                                                svg { class: "h-4 w-4 text-gray-300 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                                }
                                            }
                                            span { class: if password_validation().has_uppercase { "text-green-700" } else { "text-gray-500" }, "包含大写字母" }
                                        }
                                        div { class: "flex items-center text-xs",
                                            if password_validation().has_lowercase {
                                                svg { class: "h-4 w-4 text-green-500 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z", clip_rule: "evenodd" }
                                                }
                                            } else {
                                                svg { class: "h-4 w-4 text-gray-300 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                                }
                                            }
                                            span { class: if password_validation().has_lowercase { "text-green-700" } else { "text-gray-500" }, "包含小写字母" }
                                        }
                                        div { class: "flex items-center text-xs",
                                            if password_validation().has_digit {
                                                svg { class: "h-4 w-4 text-green-500 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z", clip_rule: "evenodd" }
                                                }
                                            } else {
                                                svg { class: "h-4 w-4 text-gray-300 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                                }
                                            }
                                            span { class: if password_validation().has_digit { "text-green-700" } else { "text-gray-500" }, "包含数字" }
                                        }
                                        div { class: "flex items-center text-xs",
                                            if password_validation().has_special {
                                                svg { class: "h-4 w-4 text-green-500 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z", clip_rule: "evenodd" }
                                                }
                                            } else {
                                                svg { class: "h-4 w-4 text-gray-300 mr-1", xmlns: "http://www.w3.org/2000/svg", view_box: "0 0 20 20", fill: "currentColor",
                                                    path { fill_rule: "evenodd", d: "M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z", clip_rule: "evenodd" }
                                                }
                                            }
                                            span { class: if password_validation().has_special { "text-green-700" } else { "text-gray-500" }, "包含特殊字符" }
                                        }
                                    }
                                }

                                // Confirm password
                                div { class: "mb-4",
                                    label { class: "block text-sm font-medium text-gray-700",
                                        "确认密码"
                                    }
                                    div { class: "mt-1",
                                        input {
                                            r#type: "password",
                                            value: "{confirm_password}",
                                            oninput: move |e| confirm_password.set(e.value().clone()),
                                            class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                            placeholder: "再次输入密码",
                                        }
                                    }
                                    if !confirm_password().is_empty() && password() != confirm_password() {
                                        p { class: "mt-1 text-xs text-red-600", "密码不匹配" }
                                    }
                                }

                                // Submit button
                                div { class: "mt-5",
                                    button {
                                        r#type: "submit",
                                        disabled: is_loading() || !password_validation().is_valid() || password() != confirm_password(),
                                        class: "w-full inline-flex justify-center rounded-md border border-transparent shadow-sm px-4 py-2 bg-blue-600 text-base font-medium text-white hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed sm:text-sm",
                                        if is_loading() {
                                            "创建中..."
                                        } else {
                                            "创建管理员"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Created credentials display
                    if let Some(creds) = created_credentials() {
                        div { class: "bg-white shadow rounded-lg",
                            div { class: "px-4 py-5 sm:p-6",
                                h4 { class: "text-lg font-medium text-gray-900 mb-4",
                                    "管理员已创建"
                                }
                                div { class: "space-y-4",
                                    // User ID
                                    div { class: "flex items-center justify-between",
                                        div { class: "flex-1",
                                            label { class: "block text-sm font-medium text-gray-700", "用户 ID" }
                                            div { class: "mt-1 flex-1 block w-full px-3 py-2 bg-gray-50 border border-gray-300 rounded-md text-sm text-gray-900",
                                                "{creds.user_id}"
                                            }
                                        }
                                        button {
                                            onclick: move |_| copy_to_clipboard(creds.user_id.clone()),
                                            class: "ml-3 inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50",
                                            "复制"
                                        }
                                    }

                                    // Username
                                    div { class: "flex items-center justify-between",
                                        div { class: "flex-1",
                                            label { class: "block text-sm font-medium text-gray-700", "用户名" }
                                            div { class: "mt-1 flex-1 block w-full px-3 py-2 bg-gray-50 border border-gray-300 rounded-md text-sm text-gray-900",
                                                "{creds.username}"
                                            }
                                        }
                                        button {
                                            onclick: move |_| copy_to_clipboard(creds.username.clone()),
                                            class: "ml-3 inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50",
                                            "复制"
                                        }
                                    }

                                    // Password
                                    div { class: "flex items-center justify-between",
                                        div { class: "flex-1",
                                            label { class: "block text-sm font-medium text-gray-700", "密码" }
                                            div { class: "mt-1 flex-1 block w-full px-3 py-2 bg-gray-50 border border-gray-300 rounded-md text-sm text-gray-900 font-mono",
                                                "{creds.password}"
                                            }
                                        }
                                        button {
                                            onclick: move |_| copy_to_clipboard(creds.password.clone()),
                                            class: "ml-3 inline-flex items-center px-3 py-2 border border-gray-300 shadow-sm text-sm leading-4 font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50",
                                            "复制"
                                        }
                                    }
                                }

                                div { class: "mt-4 p-4 bg-yellow-50 rounded-md",
                                    p { class: "text-sm text-yellow-800",
                                        "请妥善保管这些凭据。密码仅显示一次，建议在首次登录后立即更改密码。"
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // Loading state
                div { class: "bg-white shadow rounded-lg",
                    div { class: "px-4 py-5 sm:p-6",
                        div { class: "flex justify-center py-8",
                            div { class: "animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" }
                        }
                    }
                }
            }
        }
    }
}