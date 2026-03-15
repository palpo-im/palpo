//! Server status page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Server status information
#[derive(Clone, Debug, PartialEq)]
struct ServerStatusInfo {
    status: String,
    version: String,
    uptime_seconds: u64,
    memory_usage: u64,
    cpu_usage: f64,
    active_connections: u32,
    database_status: String,
    federation_status: String,
}

/// Server notification information
#[derive(Clone, Debug, PartialEq)]
struct ServerNotification {
    id: String,
    level: NotificationLevel,
    title: String,
    message: String,
    timestamp: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum NotificationLevel {
    Info,
    Warning,
    Error,
}

/// Server status dashboard component
#[component]
pub fn ServerStatusPage() -> Element {
    // State management
    let mut status_info = use_signal(|| None::<ServerStatusInfo>);
    let notifications = use_signal(|| Vec::<ServerNotification>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    
    // Load server status effect (placeholder - will be implemented in Phase 4)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 4
        // Mock data for UI testing
        let mock_status = ServerStatusInfo {
            status: "running".to_string(),
            version: "0.1.0".to_string(),
            uptime_seconds: 86400,
            memory_usage: 512 * 1024 * 1024, // 512 MB
            cpu_usage: 15.5,
            active_connections: 42,
            database_status: "healthy".to_string(),
            federation_status: "connected".to_string(),
        };
        status_info.set(Some(mock_status));
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header
            div {
                h2 { class: "text-2xl font-bold text-gray-900", "服务器状态" }
                p { class: "mt-1 text-sm text-gray-500", "监控 Palpo Matrix 服务器运行状态" }
            }

            // Loading state
            if loading() {
                div { class: "bg-white shadow rounded-lg p-12",
                    Spinner { size: "large".to_string(), message: Some("加载服务器状态...".to_string()) }
                }
            } else if let Some(err) = error() {
                div { class: "bg-white shadow rounded-lg p-6",
                    ErrorMessage { message: err }
                }
            } else if let Some(status) = status_info() {
                // Status overview cards
                div { class: "grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4",
                    StatusCard {
                        title: "服务器状态",
                        value: status.status.clone(),
                        icon: "🟢",
                        color: "green"
                    }
                    StatusCard {
                        title: "运行时长",
                        value: format_uptime(status.uptime_seconds),
                        icon: "⏱️",
                        color: "blue"
                    }
                    StatusCard {
                        title: "活跃连接",
                        value: format!("{}", status.active_connections),
                        icon: "🔌",
                        color: "purple"
                    }
                    StatusCard {
                        title: "版本",
                        value: status.version.clone(),
                        icon: "📦",
                        color: "gray"
                    }
                }

                // Detailed metrics
                div { class: "bg-white shadow rounded-lg",
                    div { class: "px-6 py-4 border-b border-gray-200",
                        h3 { class: "text-lg font-medium text-gray-900", "系统指标" }
                    }
                    div { class: "px-6 py-4",
                        div { class: "grid grid-cols-1 md:grid-cols-2 gap-6",
                            // Memory usage
                            div {
                                div { class: "flex items-center justify-between mb-2",
                                    span { class: "text-sm font-medium text-gray-700", "内存使用" }
                                    span { class: "text-sm text-gray-900", "{format_size(status.memory_usage)}" }
                                }
                                div { class: "w-full bg-gray-200 rounded-full h-2",
                                    div {
                                        class: "bg-blue-600 h-2 rounded-full",
                                        style: "width: 50%"
                                    }
                                }
                            }

                            // CPU usage
                            div {
                                div { class: "flex items-center justify-between mb-2",
                                    span { class: "text-sm font-medium text-gray-700", "CPU 使用率" }
                                    span { class: "text-sm text-gray-900", "{status.cpu_usage:.1}%" }
                                }
                                div { class: "w-full bg-gray-200 rounded-full h-2",
                                    div {
                                        class: "bg-green-600 h-2 rounded-full",
                                        style: "width: {status.cpu_usage}%"
                                    }
                                }
                            }
                        }
                    }
                }

                // Component status
                div { class: "bg-white shadow rounded-lg",
                    div { class: "px-6 py-4 border-b border-gray-200",
                        h3 { class: "text-lg font-medium text-gray-900", "组件状态" }
                    }
                    div { class: "px-6 py-4",
                        div { class: "space-y-3",
                            ComponentStatus {
                                name: "数据库",
                                status: status.database_status.clone(),
                                icon: "🗄️"
                            }
                            ComponentStatus {
                                name: "联邦服务",
                                status: status.federation_status.clone(),
                                icon: "🌐"
                            }
                        }
                    }
                }
            }

            // Server notifications
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-6 py-4 border-b border-gray-200",
                    h3 { class: "text-lg font-medium text-gray-900", "服务器通知" }
                }
                div { class: "divide-y divide-gray-200",
                    if notifications().is_empty() {
                        div { class: "px-6 py-12 text-center",
                            p { class: "text-gray-500", "暂无服务器通知" }
                        }
                    } else {
                        for notification in notifications() {
                            NotificationItem {
                                key: "{notification.id}",
                                notification: notification.clone()
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Status card component
#[component]
fn StatusCard(title: String, value: String, icon: String, color: String) -> Element {
    let bg_color = match color.as_str() {
        "green" => "bg-green-50",
        "blue" => "bg-blue-50",
        "purple" => "bg-purple-50",
        "gray" => "bg-gray-50",
        _ => "bg-gray-50",
    };
    
    let text_color = match color.as_str() {
        "green" => "text-green-600",
        "blue" => "text-blue-600",
        "purple" => "text-purple-600",
        "gray" => "text-gray-600",
        _ => "text-gray-600",
    };

    rsx! {
        div { class: "bg-white shadow rounded-lg p-6",
            div { class: "flex items-center",
                div { class: "flex-shrink-0 {bg_color} rounded-md p-3",
                    span { class: "text-2xl", "{icon}" }
                }
                div { class: "ml-4 flex-1",
                    p { class: "text-sm font-medium text-gray-500", "{title}" }
                    p { class: "mt-1 text-2xl font-semibold {text_color}", "{value}" }
                }
            }
        }
    }
}

/// Component status item
#[component]
fn ComponentStatus(name: String, status: String, icon: String) -> Element {
    let (badge_class, status_text) = match status.as_str() {
        "healthy" => ("bg-green-100 text-green-800", "正常"),
        "degraded" => ("bg-yellow-100 text-yellow-800", "降级"),
        "error" => ("bg-red-100 text-red-800", "错误"),
        _ => ("bg-gray-100 text-gray-800", "未知"),
    };

    rsx! {
        div { class: "flex items-center justify-between",
            div { class: "flex items-center gap-3",
                span { class: "text-2xl", "{icon}" }
                span { class: "text-sm font-medium text-gray-900", "{name}" }
            }
            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium {badge_class}",
                "{status_text}"
            }
        }
    }
}

/// Notification item component
#[component]
fn NotificationItem(notification: ServerNotification) -> Element {
    let (icon, bg_class, border_class) = match notification.level {
        NotificationLevel::Info => ("ℹ️", "bg-blue-50", "border-blue-200"),
        NotificationLevel::Warning => ("⚠️", "bg-yellow-50", "border-yellow-200"),
        NotificationLevel::Error => ("❌", "bg-red-50", "border-red-200"),
    };

    rsx! {
        div { class: "px-6 py-4",
            div { class: "flex items-start gap-3",
                span { class: "text-xl flex-shrink-0", "{icon}" }
                div { class: "flex-1 min-w-0",
                    div { class: "flex items-center justify-between",
                        h4 { class: "text-sm font-medium text-gray-900", "{notification.title}" }
                        span { class: "text-xs text-gray-500", "{format_timestamp(notification.timestamp)}" }
                    }
                    p { class: "mt-1 text-sm text-gray-600", "{notification.message}" }
                }
            }
        }
    }
}

/// Format uptime in human-readable format
fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    
    if days > 0 {
        format!("{}天 {}小时", days, hours)
    } else if hours > 0 {
        format!("{}小时 {}分钟", hours, minutes)
    } else {
        format!("{}分钟", minutes)
    }
}

/// Format file size in human-readable format
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format Unix timestamp to readable date string
fn format_timestamp(ts: u64) -> String {
    use chrono::{Utc, TimeZone};
    
    let dt = Utc.timestamp_opt(ts as i64, 0).single();
    match dt {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M").to_string(),
        None => "无效时间".to_string(),
    }
}
