//! User media statistics page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// User media statistics information
#[derive(Clone, Debug, PartialEq)]
struct UserMediaStats {
    user_id: String,
    username: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    media_count: u64,
    media_length: u64,
}

/// User media statistics manager component with list, search, and batch operations
#[component]
pub fn UserMediaStatsManager() -> Element {
    // State management
    let user_stats = use_signal(|| Vec::<UserMediaStats>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut search_query = use_signal(|| String::new());
    let mut current_page = use_signal(|| 0u32);
    let total_count = use_signal(|| 0u32);
    let mut selected_users = use_signal(|| Vec::<String>::new());
    let mut show_batch_menu = use_signal(|| false);
    
    let page_size = 20u32;
    let total_pages = (total_count() + page_size - 1) / page_size;

    // Load user media stats effect (placeholder - will be implemented in Phase 3)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 3
        loading.set(false);
    });

    rsx! {
        div { class: "p-4 sm:p-6 space-y-6",
            // Header with actions
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "用户媒体统计" }
                    p { class: "mt-1 text-sm text-gray-500", "查看用户媒体使用情况和存储统计" }
                }
            }

            // Search bar
            div { class: "bg-white shadow rounded-lg p-4",
                div { class: "grid grid-cols-1 gap-4",
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "搜索用户" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            placeholder: "按用户ID或显示名搜索...",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value())
                        }
                    }
                }
            }

            // Batch operations bar
            if !selected_users().is_empty() {
                div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                    div { class: "flex items-center justify-between",
                        div { class: "flex items-center gap-2",
                            span { class: "text-sm font-medium text-blue-900",
                                "已选择 {selected_users().len()} 个用户"
                            }
                            button {
                                class: "text-sm text-blue-600 hover:text-blue-800 underline",
                                onclick: move |_| selected_users.set(Vec::new()),
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
                                            class: "block w-full text-left px-4 py-2 text-sm text-red-700 hover:bg-red-50",
                                            onclick: move |_| {
                                                // TODO: Implement batch delete media
                                                show_batch_menu.set(false);
                                            },
                                            "🗑️ 删除用户媒体"
                                        }
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                                            onclick: move |_| {
                                                // TODO: Implement batch clean remote media
                                                show_batch_menu.set(false);
                                            },
                                            "🧹 清理远程媒体"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // User stats list table
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载用户媒体统计...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if user_stats().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "📊" }
                        p { class: "text-gray-500 text-lg", "暂无用户媒体统计" }
                        p { class: "text-gray-400 text-sm mt-2", "当前没有找到任何用户媒体数据" }
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
                                            checked: !user_stats().is_empty() && selected_users().len() == user_stats().len(),
                                            onchange: move |evt| {
                                                if evt.checked() {
                                                    selected_users.set(user_stats().iter().map(|u| u.user_id.clone()).collect());
                                                } else {
                                                    selected_users.set(Vec::new());
                                                }
                                            }
                                        }
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "用户" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "媒体数量" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "总大小" }
                                    th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                                }
                            }
                            tbody { class: "bg-white divide-y divide-gray-200",
                                for stats in user_stats() {
                                    UserStatsRow {
                                        key: "{stats.user_id}",
                                        stats: stats.clone(),
                                        selected: selected_users().contains(&stats.user_id),
                                        on_select: move |user_id: String| {
                                            let mut current = selected_users();
                                            if current.contains(&user_id) {
                                                current.retain(|id| id != &user_id);
                                            } else {
                                                current.push(user_id);
                                            }
                                            selected_users.set(current);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Pagination
                    div { class: "px-6 py-4 border-t border-gray-200 flex items-center justify-between",
                        div { class: "text-sm text-gray-700",
                            "显示 {current_page() * page_size + 1} - {((current_page() + 1) * page_size).min(total_count())} / 共 {total_count()} 个用户"
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

/// Individual user stats row component
#[component]
fn UserStatsRow(
    stats: UserMediaStats,
    selected: bool,
    on_select: EventHandler<String>,
) -> Element {
    let user_id = stats.user_id.clone();
    
    rsx! {
        tr { class: if selected { "bg-blue-50" } else { "hover:bg-gray-50" },
            td { class: "px-6 py-4 whitespace-nowrap",
                input {
                    r#type: "checkbox",
                    class: "rounded border-gray-300 text-blue-600 focus:ring-blue-500",
                    checked: selected,
                    onchange: move |_| on_select.call(user_id.clone())
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "flex items-center",
                    div { class: "flex-shrink-0 h-10 w-10",
                        if let Some(avatar_url) = &stats.avatar_url {
                            img {
                                class: "h-10 w-10 rounded-full",
                                src: "{avatar_url}",
                                alt: "{stats.username}"
                            }
                        } else {
                            div { class: "h-10 w-10 rounded-full bg-gray-300 flex items-center justify-center text-gray-600 font-semibold",
                                "{stats.username.chars().next().unwrap_or('U').to_uppercase()}"
                            }
                        }
                    }
                    div { class: "ml-4",
                        div { class: "text-sm font-medium text-gray-900", "{stats.username}" }
                        div { class: "text-xs text-gray-500", "{stats.user_id}" }
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-900",
                "{stats.media_count}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-900",
                "{format_size(stats.media_length)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                button {
                    class: "text-blue-600 hover:text-blue-900",
                    onclick: move |_| {
                        // TODO: Navigate to user media detail
                    },
                    "查看详情"
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
