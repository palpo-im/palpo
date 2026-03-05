//! Appservice management page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Appservice information
#[derive(Clone, Debug, PartialEq)]
struct Appservice {
    id: String,
    url: String,
    hs_token: String,
    as_token: String,
    sender_localpart: String,
    namespaces: AppserviceNamespaces,
    is_active: bool,
    created_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct AppserviceNamespaces {
    users: Vec<String>,
    aliases: Vec<String>,
    rooms: Vec<String>,
}

/// Appservice manager component with list, registration, and testing functionality
#[component]
pub fn AppserviceManager() -> Element {
    // State management
    let appservices = use_signal(|| Vec::<Appservice>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut show_register_dialog = use_signal(|| false);
    let mut yaml_content = use_signal(String::new);
    
    // Load appservices effect (placeholder - will be implemented in Phase 4)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 4
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header with actions
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "应用服务管理" }
                    p { class: "mt-1 text-sm text-gray-500", "管理 Matrix 应用服务 (Appservices)" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                        onclick: move |_| show_register_dialog.set(true),
                        "➕ 注册 Appservice"
                    }
                }
            }

            // Info banner
            div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                div { class: "flex",
                    div { class: "flex-shrink-0",
                        span { class: "text-blue-400 text-xl", "ℹ️" }
                    }
                    div { class: "ml-3",
                        h3 { class: "text-sm font-medium text-blue-800", "关于应用服务" }
                        p { class: "mt-2 text-sm text-blue-700",
                            "应用服务 (Appservices) 允许第三方服务和机器人与 Matrix 服务器集成。通过上传 YAML 配置文件来注册新的应用服务。"
                        }
                    }
                }
            }

            // Appservices list
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载应用服务列表...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if appservices().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "🤖" }
                        p { class: "text-gray-500 text-lg", "暂无应用服务" }
                        p { class: "text-gray-400 text-sm mt-2", "点击上方\"注册 Appservice\"按钮添加第一个应用服务" }
                    }
                } else {
                    div { class: "divide-y divide-gray-200",
                        for appservice in appservices() {
                            AppserviceCard {
                                key: "{appservice.id}",
                                appservice: appservice.clone()
                            }
                        }
                    }
                }
            }

            // Register dialog
            if show_register_dialog() {
                RegisterAppserviceDialog {
                    yaml_content: yaml_content(),
                    on_yaml_change: move |value: String| yaml_content.set(value),
                    on_register: move |_| {
                        // TODO: Implement register in Phase 4
                        show_register_dialog.set(false);
                    },
                    on_cancel: move |_| {
                        show_register_dialog.set(false);
                        yaml_content.set(String::new());
                    }
                }
            }
        }
    }
}

/// Individual appservice card component
#[component]
fn AppserviceCard(appservice: Appservice) -> Element {
    let mut show_details = use_signal(|| false);
    
    rsx! {
        div { class: "p-6 hover:bg-gray-50",
            div { class: "flex items-start justify-between",
                div { class: "flex-1",
                    div { class: "flex items-center gap-3",
                        div { class: "flex-shrink-0",
                            div { class: "h-12 w-12 rounded-lg bg-blue-100 flex items-center justify-center text-blue-600 text-2xl",
                                "🤖"
                            }
                        }
                        div {
                            h3 { class: "text-lg font-medium text-gray-900", "{appservice.id}" }
                            p { class: "text-sm text-gray-500", "{appservice.url}" }
                        }
                        if appservice.is_active {
                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                                "🟢 活跃"
                            }
                        } else {
                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                                "⚪ 未激活"
                            }
                        }
                    }
                    
                    if show_details() {
                        div { class: "mt-4 space-y-3",
                            div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                                InfoItem { label: "Sender Localpart", value: appservice.sender_localpart.clone() }
                                InfoItem { label: "创建时间", value: format_timestamp(appservice.created_at) }
                            }
                            
                            div { class: "mt-4",
                                h4 { class: "text-sm font-medium text-gray-700 mb-2", "命名空间" }
                                div { class: "grid grid-cols-1 md:grid-cols-3 gap-4",
                                    div {
                                        p { class: "text-xs font-medium text-gray-500 mb-1", "用户" }
                                        div { class: "space-y-1",
                                            if appservice.namespaces.users.is_empty() {
                                                p { class: "text-xs text-gray-400", "无" }
                                            } else {
                                                for pattern in &appservice.namespaces.users {
                                                    p { class: "text-xs text-gray-600 font-mono", "{pattern}" }
                                                }
                                            }
                                        }
                                    }
                                    div {
                                        p { class: "text-xs font-medium text-gray-500 mb-1", "别名" }
                                        div { class: "space-y-1",
                                            if appservice.namespaces.aliases.is_empty() {
                                                p { class: "text-xs text-gray-400", "无" }
                                            } else {
                                                for pattern in &appservice.namespaces.aliases {
                                                    p { class: "text-xs text-gray-600 font-mono", "{pattern}" }
                                                }
                                            }
                                        }
                                    }
                                    div {
                                        p { class: "text-xs font-medium text-gray-500 mb-1", "房间" }
                                        div { class: "space-y-1",
                                            if appservice.namespaces.rooms.is_empty() {
                                                p { class: "text-xs text-gray-400", "无" }
                                            } else {
                                                for pattern in &appservice.namespaces.rooms {
                                                    p { class: "text-xs text-gray-600 font-mono", "{pattern}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                div { class: "flex gap-2",
                    button {
                        class: "text-sm text-blue-600 hover:text-blue-900",
                        onclick: move |_| show_details.set(!show_details()),
                        if show_details() { "隐藏详情" } else { "显示详情" }
                    }
                    button {
                        class: "text-sm text-green-600 hover:text-green-900",
                        onclick: move |_| {
                            // TODO: Implement test
                        },
                        "测试"
                    }
                    button {
                        class: "text-sm text-red-600 hover:text-red-900",
                        onclick: move |_| {
                            // TODO: Implement unregister
                        },
                        "注销"
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
        div { class: "bg-gray-50 px-3 py-2 rounded-md",
            dt { class: "text-xs font-medium text-gray-500", "{label}" }
            dd { class: "mt-1 text-sm text-gray-900", "{value}" }
        }
    }
}

/// Register appservice dialog component
#[component]
fn RegisterAppserviceDialog(
    yaml_content: String,
    on_yaml_change: EventHandler<String>,
    on_register: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "fixed inset-0 bg-gray-500 bg-opacity-75 flex items-center justify-center z-50 p-4",
            div { class: "bg-white rounded-lg shadow-xl max-w-2xl w-full max-h-[90vh] flex flex-col",
                div { class: "px-6 py-4 border-b border-gray-200",
                    h3 { class: "text-lg font-medium text-gray-900",
                        "注册应用服务"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "上传 YAML 配置文件来注册新的应用服务"
                    }
                }
                div { class: "px-6 py-4 flex-1 overflow-y-auto",
                    div { class: "space-y-4",
                        div {
                            label { class: "block text-sm font-medium text-gray-700 mb-2",
                                "YAML 配置"
                            }
                            textarea {
                                class: "w-full h-96 px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 font-mono text-sm",
                                placeholder: "id: my-appservice\nurl: http://localhost:8080\nhs_token: ...\nas_token: ...\nsender_localpart: bot\nnamespaces:\n  users:\n    - exclusive: true\n      regex: '@bot_.*'",
                                value: "{yaml_content}",
                                oninput: move |evt| on_yaml_change.call(evt.value())
                            }
                        }
                        
                        div { class: "bg-yellow-50 border border-yellow-200 rounded-md p-3",
                            p { class: "text-xs text-yellow-800",
                                "⚠️ 请确保 YAML 配置格式正确。无效的配置将导致注册失败。"
                            }
                        }
                    }
                }
                div { class: "px-6 py-4 bg-gray-50 flex justify-end gap-3 rounded-b-lg border-t border-gray-200",
                    button {
                        class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50",
                        onclick: move |_| on_cancel.call(()),
                        "取消"
                    }
                    button {
                        class: "px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700 disabled:opacity-50",
                        disabled: yaml_content.is_empty(),
                        onclick: move |_| on_register.call(()),
                        "注册"
                    }
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