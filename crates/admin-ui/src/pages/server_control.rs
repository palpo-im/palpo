//! Server Control Page Component
//!
//! This module provides a web interface for controlling the Palpo Matrix server lifecycle.
//! It allows Web UI admins to start, stop, restart the server and view its current status.
//!
//! # Features
//!
//! - Real-time server status display with auto-refresh
//! - Start/Stop/Restart buttons with confirmation dialogs
//! - Server uptime and process ID display
//! - Error handling with clear user feedback
//! - Status polling every 3 seconds
//!
//! # Requirements
//!
//! Implements requirements:
//! - 6.1: Display current server status
//! - 6.2: Start server button
//! - 6.3: Stop server button
//! - 6.4: Restart server button
//! - 6.5: Show server logs and errors
//! - 6.6: Poll status for updates

use dioxus::prelude::*;
use crate::services::api_client::get_api_client;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

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

impl ServerStatus {
    /// Get display text for status
    pub fn display_text(&self) -> &'static str {
        match self {
            ServerStatus::NotStarted => "未启动",
            ServerStatus::Starting => "启动中...",
            ServerStatus::Running => "运行中",
            ServerStatus::Stopping => "停止中...",
            ServerStatus::Stopped => "已停止",
            ServerStatus::Error => "错误",
        }
    }

    /// Get CSS class for status badge
    pub fn badge_class(&self) -> &'static str {
        match self {
            ServerStatus::NotStarted => "bg-gray-100 text-gray-800",
            ServerStatus::Starting => "bg-yellow-100 text-yellow-800",
            ServerStatus::Running => "bg-green-100 text-green-800",
            ServerStatus::Stopping => "bg-orange-100 text-orange-800",
            ServerStatus::Stopped => "bg-gray-100 text-gray-800",
            ServerStatus::Error => "bg-red-100 text-red-800",
        }
    }
}

/// Server status information from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatusInfo {
    pub status: ServerStatus,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
    pub uptime_seconds: Option<i64>,
}

/// API response for success operations
#[derive(Debug, Deserialize)]
struct SuccessResponse {
    message: String,
}

/// Format uptime in human-readable format
fn format_uptime(seconds: i64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    
    if hours > 0 {
        format!("{}小时 {}分钟 {}秒", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}分钟 {}秒", minutes, secs)
    } else {
        format!("{}秒", secs)
    }
}

/// Main server control page component
///
/// This component provides a complete interface for managing the Palpo server lifecycle.
/// It polls the server status every 3 seconds and provides buttons for start/stop/restart
/// operations with confirmation dialogs.
///
/// # State Management
///
/// - `status_info`: Current server status information
/// - `is_loading`: Whether an operation is in progress
/// - `error_message`: Error message to display
/// - `success_message`: Success message to display
/// - `show_start_confirm`: Whether to show start confirmation dialog
/// - `show_stop_confirm`: Whether to show stop confirmation dialog
/// - `show_restart_confirm`: Whether to show restart confirmation dialog
#[component]
pub fn ServerControlPage() -> Element {
    let mut status_info = use_signal(|| None::<ServerStatusInfo>);
    let mut is_loading = use_signal(|| false);
    let mut error_message = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);
    let mut show_start_confirm = use_signal(|| false);
    let mut show_stop_confirm = use_signal(|| false);
    let mut show_restart_confirm = use_signal(|| false);

    // Fetch server status
    let fetch_status = use_callback(move |_| async move {
        match get_api_client() {
            Ok(client) => {
                match client.get_json::<ServerStatusInfo>("/api/v1/admin/server/status").await {
                    Ok(info) => {
                        status_info.set(Some(info));
                    }
                    Err(e) => {
                        web_sys::console::error_1(&format!("Failed to fetch status: {}", e).into());
                    }
                }
            }
            Err(e) => {
                web_sys::console::error_1(&format!("API client error: {}", e).into());
            }
        }
    });

    // Poll status every 3 seconds
    use_effect(move || {
        spawn(async move {
            loop {
                let _ = fetch_status.call(());
                #[cfg(target_arch = "wasm32")]
                {
                    gloo_timers::future::sleep(std::time::Duration::from_secs(3)).await;
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                }
            }
        });
    });

    // Handle start server
    let handle_start = move |_| {
        spawn(async move {
            is_loading.set(true);
            error_message.set(None);
            success_message.set(None);
            show_start_confirm.set(false);

            match get_api_client() {
                Ok(client) => {
                    match client.post_json_response::<(), SuccessResponse>("/api/v1/admin/server/start", &()).await {
                        Ok(resp) => {
                            success_message.set(Some(resp.message));
                            let _ = fetch_status.call(());
                        }
                        Err(e) => {
                            error_message.set(Some(format!("启动服务器失败: {}", e)));
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

    // Handle stop server
    let handle_stop = move |_| {
        spawn(async move {
            is_loading.set(true);
            error_message.set(None);
            success_message.set(None);
            show_stop_confirm.set(false);

            match get_api_client() {
                Ok(client) => {
                    match client.post_json_response::<(), SuccessResponse>("/api/v1/admin/server/stop", &()).await {
                        Ok(resp) => {
                            success_message.set(Some(resp.message));
                            let _ = fetch_status.call(());
                        }
                        Err(e) => {
                            error_message.set(Some(format!("停止服务器失败: {}", e)));
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

    // Handle restart server
    let handle_restart = move |_| {
        spawn(async move {
            is_loading.set(true);
            error_message.set(None);
            success_message.set(None);
            show_restart_confirm.set(false);

            match get_api_client() {
                Ok(client) => {
                    match client.post_json_response::<(), SuccessResponse>("/api/v1/admin/server/restart", &()).await {
                        Ok(resp) => {
                            success_message.set(Some(resp.message));
                            let _ = fetch_status.call(());
                        }
                        Err(e) => {
                            error_message.set(Some(format!("重启服务器失败: {}", e)));
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

    rsx! {
        div { class: "space-y-6",
            // Header
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "服务器控制"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Palpo Matrix 服务器生命周期"
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

            // Server Status Card
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h4 { class: "text-base font-medium text-gray-900 mb-4",
                        "服务器状态"
                    }

                    if let Some(info) = status_info() {
                        div { class: "space-y-4",
                            // Status badge
                            div { class: "flex items-center space-x-3",
                                span { class: "text-sm font-medium text-gray-700",
                                    "状态:"
                                }
                                span {
                                    class: "px-3 py-1 rounded-full text-sm font-medium {info.status.badge_class()}",
                                    "{info.status.display_text()}"
                                }
                            }

                            // Process ID
                            if let Some(pid) = info.pid {
                                div { class: "flex items-center space-x-3",
                                    span { class: "text-sm font-medium text-gray-700",
                                        "进程 ID:"
                                    }
                                    span { class: "text-sm text-gray-900",
                                        "{pid}"
                                    }
                                }
                            }

                            // Started at
                            if let Some(started_at) = info.started_at {
                                div { class: "flex items-center space-x-3",
                                    span { class: "text-sm font-medium text-gray-700",
                                        "启动时间:"
                                    }
                                    span { class: "text-sm text-gray-900",
                                        {started_at.format("%Y-%m-%d %H:%M:%S").to_string()}
                                    }
                                }
                            }

                            // Uptime
                            if let Some(uptime) = info.uptime_seconds {
                                div { class: "flex items-center space-x-3",
                                    span { class: "text-sm font-medium text-gray-700",
                                        "运行时长:"
                                    }
                                    span { class: "text-sm text-gray-900",
                                        "{format_uptime(uptime)}"
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "flex justify-center py-8",
                            div { class: "animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600" }
                        }
                    }
                }
            }

            // Control Buttons
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h4 { class: "text-base font-medium text-gray-900 mb-4",
                        "服务器操作"
                    }

                    div { class: "flex space-x-3",
                        button {
                            class: "px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed",
                            disabled: is_loading() || status_info().map(|s| s.status == ServerStatus::Running).unwrap_or(false),
                            onclick: move |_| show_start_confirm.set(true),
                            "启动服务器"
                        }

                        button {
                            class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed",
                            disabled: is_loading() || status_info().map(|s| s.status != ServerStatus::Running).unwrap_or(true),
                            onclick: move |_| show_stop_confirm.set(true),
                            "停止服务器"
                        }

                        button {
                            class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed",
                            disabled: is_loading() || status_info().map(|s| s.status != ServerStatus::Running).unwrap_or(true),
                            onclick: move |_| show_restart_confirm.set(true),
                            "重启服务器"
                        }
                    }
                }
            }

            // Start Confirmation Dialog
            if show_start_confirm() {
                ConfirmDialog {
                    title: "启动服务器".to_string(),
                    message: "确定要启动 Palpo 服务器吗？".to_string(),
                    confirm_text: "启动".to_string(),
                    cancel_text: "取消".to_string(),
                    on_confirm: handle_start,
                    on_cancel: move |_| show_start_confirm.set(false)
                }
            }

            // Stop Confirmation Dialog
            if show_stop_confirm() {
                ConfirmDialog {
                    title: "停止服务器".to_string(),
                    message: "确定要停止 Palpo 服务器吗？这将断开所有客户端连接。".to_string(),
                    confirm_text: "停止".to_string(),
                    cancel_text: "取消".to_string(),
                    on_confirm: handle_stop,
                    on_cancel: move |_| show_stop_confirm.set(false)
                }
            }

            // Restart Confirmation Dialog
            if show_restart_confirm() {
                ConfirmDialog {
                    title: "重启服务器".to_string(),
                    message: "确定要重启 Palpo 服务器吗？这将暂时断开所有客户端连接。".to_string(),
                    confirm_text: "重启".to_string(),
                    cancel_text: "取消".to_string(),
                    on_confirm: handle_restart,
                    on_cancel: move |_| show_restart_confirm.set(false)
                }
            }
        }
    }
}

/// Confirmation dialog component
#[component]
fn ConfirmDialog(
    title: String,
    message: String,
    confirm_text: String,
    cancel_text: String,
    on_confirm: EventHandler<MouseEvent>,
    on_cancel: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div { class: "fixed inset-0 bg-gray-500 bg-opacity-75 flex items-center justify-center z-50",
            div { class: "bg-white rounded-lg shadow-xl max-w-md w-full mx-4",
                div { class: "px-6 py-4 border-b border-gray-200",
                    h3 { class: "text-lg font-medium text-gray-900",
                        "{title}"
                    }
                }
                div { class: "px-6 py-4",
                    p { class: "text-sm text-gray-700",
                        "{message}"
                    }
                }
                div { class: "px-6 py-4 bg-gray-50 flex justify-end space-x-3 rounded-b-lg",
                    button {
                        class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50",
                        onclick: move |evt| on_cancel.call(evt),
                        "{cancel_text}"
                    }
                    button {
                        class: "px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700",
                        onclick: move |evt| on_confirm.call(evt),
                        "{confirm_text}"
                    }
                }
            }
        }
    }
}
