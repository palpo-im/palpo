//! Media management page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Media file information
#[derive(Clone, Debug, PartialEq)]
struct MediaFile {
    mxc_uri: String,
    content_type: String,
    size: u64,
    uploader: String,
    created_at: u64,
    is_quarantined: bool,
    is_protected: bool,
}

/// Media manager component with list, search, filter, and batch operations
#[component]
pub fn MediaManager() -> Element {
    // State management
    let media_files = use_signal(|| Vec::<MediaFile>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut search_query = use_signal(|| String::new());
    let mut filter_quarantined = use_signal(|| None::<bool>);
    let mut filter_content_type = use_signal(|| String::from("all"));
    let mut current_page = use_signal(|| 0u32);
    let total_count = use_signal(|| 0u32);
    let mut selected_media = use_signal(|| Vec::<String>::new());
    let mut show_batch_menu = use_signal(|| false);
    
    let page_size = 20u32;
    let total_pages = (total_count() + page_size - 1) / page_size;

    // Load media files effect (placeholder - will be implemented in Phase 3)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 3
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header with actions
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "媒体管理" }
                    p { class: "mt-1 text-sm text-gray-500", "管理 Matrix 媒体文件和存储" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "inline-flex items-center px-4 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                        onclick: move |_| {
                            // TODO: Navigate to user media stats page
                        },
                        "📊 用户媒体统计"
                    }
                }
            }

            // Search and filters
            div { class: "bg-white shadow rounded-lg p-4",
                div { class: "grid grid-cols-1 md:grid-cols-4 gap-4",
                    // Search input
                    div { class: "md:col-span-2",
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "搜索媒体" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            placeholder: "按 MXC URI 或上传者搜索...",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value())
                        }
                    }

                    // Content type filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "内容类型" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: "{filter_content_type}",
                            onchange: move |evt| filter_content_type.set(evt.value()),
                            option { value: "all", "全部" }
                            option { value: "image", "图片" }
                            option { value: "video", "视频" }
                            option { value: "audio", "音频" }
                            option { value: "document", "文档" }
                        }
                    }

                    // Quarantined filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "隔离状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_quarantined() {
                                None => "all",
                                Some(true) => "quarantined",
                                Some(false) => "normal",
                            },
                            onchange: move |evt| {
                                filter_quarantined.set(match evt.value().as_str() {
                                    "all" => None,
                                    "quarantined" => Some(true),
                                    "normal" => Some(false),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "normal", "正常" }
                            option { value: "quarantined", "已隔离" }
                        }
                    }
                }
            }

            // Batch operations bar
            if !selected_media().is_empty() {
                div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                    div { class: "flex items-center justify-between",
                        div { class: "flex items-center gap-2",
                            span { class: "text-sm font-medium text-blue-900",
                                "已选择 {selected_media().len()} 个媒体文件"
                            }
                            button {
                                class: "text-sm text-blue-600 hover:text-blue-800 underline",
                                onclick: move |_| selected_media.set(Vec::new()),
                                "清除选择"
                            }
                        }
                        div { class: "relative",
                            button {
                                class: "inline-flex items-center px-4 py-2 border border-blue-300 text-sm font-medium rounded-md text-blue-700 bg-white hover:bg-blue-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                                onclick: move |_| show_batch_menu.set(!show_batch_menu()),
                                "批量操作 ▼"
                            }
                            if show_batch_menu() {
                                div { class: "absolute right-0 mt-2 w-56 rounded-md shadow-lg bg-white ring-1 ring-black ring-opacity-5 z-10",
                                    div { class: "py-1",
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                                            onclick: move |_| {
                                                // TODO: Implement batch quarantine
                                                show_batch_menu.set(false);
                                            },
                                            "🔒 隔离媒体"
                                        }
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                                            onclick: move |_| {
                                                // TODO: Implement batch protect
                                                show_batch_menu.set(false);
                                            },
                                            "🛡️ 保护媒体"
                                        }
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-red-700 hover:bg-red-50",
                                            onclick: move |_| {
                                                // TODO: Implement batch delete
                                                show_batch_menu.set(false);
                                            },
                                            "🗑️ 删除媒体"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Media list table
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载媒体列表...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if media_files().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "🖼️" }
                        p { class: "text-gray-500 text-lg", "暂无媒体文件" }
                        p { class: "text-gray-400 text-sm mt-2", "当前没有找到任何媒体文件" }
                    }
                } else {
                    div { class: "overflow-x-auto",
                        table { class: "min-w-full divide-y divide-gray-200",
                            thead { class: "bg-gray-50",
                                tr {
                                    th { class: "px-6 py-3 text-left",
                                        input {
                                            r#type: "checkbox",
                                            class: "rounded border-gray-300 text-blue-600 focus:ring-blue-500",
                                            checked: !media_files().is_empty() && selected_media().len() == media_files().len(),
                                            onchange: move |evt| {
                                                if evt.checked() {
                                                    selected_media.set(media_files().iter().map(|m| m.mxc_uri.clone()).collect());
                                                } else {
                                                    selected_media.set(Vec::new());
                                                }
                                            }
                                        }
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "MXC URI" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "类型" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "大小" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "上传者" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "创建时间" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "状态" }
                                    th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                                }
                            }
                            tbody { class: "bg-white divide-y divide-gray-200",
                                for media in media_files() {
                                    MediaRow {
                                        key: "{media.mxc_uri}",
                                        media: media.clone(),
                                        selected: selected_media().contains(&media.mxc_uri),
                                        on_select: move |mxc_uri: String| {
                                            let mut current = selected_media();
                                            if current.contains(&mxc_uri) {
                                                current.retain(|id| id != &mxc_uri);
                                            } else {
                                                current.push(mxc_uri);
                                            }
                                            selected_media.set(current);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Pagination
                    div { class: "px-6 py-4 border-t border-gray-200 flex items-center justify-between",
                        div { class: "text-sm text-gray-700",
                            "显示 {current_page() * page_size + 1} - {((current_page() + 1) * page_size).min(total_count())} / 共 {total_count()} 个媒体文件"
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

/// Individual media row component
#[component]
fn MediaRow(
    media: MediaFile,
    selected: bool,
    on_select: EventHandler<String>,
) -> Element {
    let mxc_uri = media.mxc_uri.clone();
    
    rsx! {
        tr { class: if selected { "bg-blue-50" } else { "hover:bg-gray-50" },
            td { class: "px-6 py-4 whitespace-nowrap",
                input {
                    r#type: "checkbox",
                    class: "rounded border-gray-300 text-blue-600 focus:ring-blue-500",
                    checked: selected,
                    onchange: move |_| on_select.call(mxc_uri.clone())
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "text-xs text-gray-500 font-mono truncate max-w-xs",
                    "{media.mxc_uri}"
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-900",
                "{media.content_type}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-900",
                "{format_size(media.size)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{media.uploader}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{format_timestamp(media.created_at)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "flex gap-2",
                    if media.is_quarantined {
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                            "🔒 已隔离"
                        }
                    }
                    if media.is_protected {
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-blue-100 text-blue-800",
                            "🛡️ 已保护"
                        }
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                button {
                    class: "text-blue-600 hover:text-blue-900",
                    onclick: move |_| {
                        // TODO: Implement view/download
                    },
                    "查看"
                }
            }
        }
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