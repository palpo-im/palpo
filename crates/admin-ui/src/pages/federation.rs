//! Federation destinations management page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Federation destination information
#[derive(Clone, Debug, PartialEq)]
struct FederationDestination {
    destination: String,
    retry_last_ts: u64,
    retry_interval: u64,
    failure_ts: Option<u64>,
    last_successful_stream_ordering: Option<u64>,
    is_failed: bool,
}

/// Federation destinations manager component with list, search, and reconnect functionality
#[component]
pub fn FederationManager() -> Element {
    // State management
    let destinations = use_signal(|| Vec::<FederationDestination>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut search_query = use_signal(|| String::new());
    let mut filter_failed = use_signal(|| None::<bool>);
    let mut current_page = use_signal(|| 0u32);
    let total_count = use_signal(|| 0u32);
    
    let page_size = 20u32;
    let total_pages = (total_count() + page_size - 1) / page_size;

    // Load destinations effect (placeholder - will be implemented in Phase 4)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 4
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "联邦目的地管理" }
                    p { class: "mt-1 text-sm text-gray-500", "管理 Matrix 联邦连接目的地" }
                }
            }

            // Search and filters
            div { class: "bg-white shadow rounded-lg p-4",
                div { class: "grid grid-cols-1 md:grid-cols-2 gap-4",
                    // Search input
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "搜索目的地" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            placeholder: "按目的地地址搜索...",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value())
                        }
                    }

                    // Connection status filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "连接状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_failed() {
                                None => "all",
                                Some(false) => "connected",
                                Some(true) => "failed",
                            },
                            onchange: move |evt| {
                                filter_failed.set(match evt.value().as_str() {
                                    "all" => None,
                                    "connected" => Some(false),
                                    "failed" => Some(true),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "connected", "已连接" }
                            option { value: "failed", "连接失败" }
                        }
                    }
                }
            }

            // Destinations list table
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载联邦目的地...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if destinations().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "🌐" }
                        p { class: "text-gray-500 text-lg", "暂无联邦目的地" }
                        p { class: "text-gray-400 text-sm mt-2", "当前没有找到任何联邦连接目的地" }
                    }
                } else {
                    div { class: "overflow-x-auto",
                        table { class: "min-w-full divide-y divide-gray-200",
                            thead { class: "bg-gray-50",
                                tr {
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "状态" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "目的地" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "最后重试" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "重试间隔" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "失败时间" }
                                    th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                                }
                            }
                            tbody { class: "bg-white divide-y divide-gray-200",
                                for dest in destinations() {
                                    DestinationRow {
                                        key: "{dest.destination}",
                                        destination: dest.clone()
                                    }
                                }
                            }
                        }
                    }

                    // Pagination
                    div { class: "px-6 py-4 border-t border-gray-200 flex items-center justify-between",
                        div { class: "text-sm text-gray-700",
                            "显示 {current_page() * page_size + 1} - {((current_page() + 1) * page_size).min(total_count())} / 共 {total_count()} 个目的地"
                        }
                        div { class: "flex gap-2",
                            button {
                                class: "px-3 py-1 border border-gray-300 rounded-md text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed",
                                disabled: current_page() == 0,
                                onclick: move |_| {
                                    if current_page() > 0 {
                                        current_page.set(current_page() - 1);
                                    }
                                },
                                "上一页"
                            }
                            span { class: "px-3 py-1 text-sm text-gray-700",
                                "第 {current_page() + 1} / {total_pages.max(1)} 页"
                            }
                            button {
                                class: "px-3 py-1 border border-gray-300 rounded-md text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 disabled:opacity-50 disabled:cursor-not-allowed",
                                disabled: current_page() >= total_pages.saturating_sub(1),
                                onclick: move |_| {
                                    if current_page() < total_pages - 1 {
                                        current_page.set(current_page() + 1);
                                    }
                                },
                                "下一页"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Individual destination row component
#[component]
fn DestinationRow(destination: FederationDestination) -> Element {
    rsx! {
        tr { class: "hover:bg-gray-50",
            td { class: "px-6 py-4 whitespace-nowrap",
                if destination.is_failed {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                        "🔴 失败"
                    }
                } else {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                        "🟢 正常"
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "text-sm font-medium text-gray-900", "{destination.destination}" }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{format_timestamp(destination.retry_last_ts)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{format_duration(destination.retry_interval)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                if let Some(failure_ts) = destination.failure_ts {
                    "{format_timestamp(failure_ts)}"
                } else {
                    "-"
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                div { class: "flex justify-end gap-2",
                    button {
                        class: "text-blue-600 hover:text-blue-900",
                        onclick: move |_| {
                            // TODO: Implement view details
                        },
                        "查看详情"
                    }
                    if destination.is_failed {
                        button {
                            class: "text-green-600 hover:text-green-900",
                            onclick: move |_| {
                                // TODO: Implement reconnect
                            },
                            "重新连接"
                        }
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
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M").to_string(),
        None => "无效时间".to_string(),
    }
}

/// Format duration in human-readable format
fn format_duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}秒", seconds)
    } else if seconds < 3600 {
        format!("{}分钟", seconds / 60)
    } else if seconds < 86400 {
        format!("{}小时", seconds / 3600)
    } else {
        format!("{}天", seconds / 86400)
    }
}