//! User management page component

use dioxus::prelude::*;
use crate::app::Route;
use crate::models::user::{User, UserSortField};
use crate::models::room::SortOrder;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// User manager component with list, search, filter, and batch operations
#[component]
pub fn UserManager() -> Element {
    // State management
    let mut users = use_signal(|| Vec::<User>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut search_query = use_signal(|| String::new());
    let mut filter_admin = use_signal(|| None::<bool>);
    let mut filter_deactivated = use_signal(|| Some(false));
    let mut sort_by = use_signal(|| UserSortField::Username);
    let mut sort_order = use_signal(|| SortOrder::Ascending);
    let mut current_page = use_signal(|| 0u32);
    let mut total_count = use_signal(|| 0u32);
    let mut selected_users = use_signal(|| Vec::<String>::new());
    let mut show_batch_menu = use_signal(|| false);
    
    let page_size = 20u32;
    let total_pages = (total_count() + page_size - 1) / page_size;

    // Load users effect (placeholder - will be implemented in Phase 2)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 2
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header with actions
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "用户管理" }
                    p { class: "mt-1 text-sm text-gray-500", "管理 Matrix 用户账户" }
                }
                div { class: "flex gap-2",
                    button {
                        class: "inline-flex items-center px-4 py-2 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                        onclick: move |_| {
                            // TODO: Navigate to create user page
                        },
                        "➕ 创建用户"
                    }
                }
            }

            // Search and filters
            div { class: "bg-white shadow rounded-lg p-4",
                div { class: "grid grid-cols-1 md:grid-cols-4 gap-4",
                    // Search input
                    div { class: "md:col-span-2",
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "搜索用户" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            placeholder: "按用户名或显示名搜索...",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value())
                        }
                    }

                    // Admin filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "管理员状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_admin() {
                                None => "all",
                                Some(true) => "admin",
                                Some(false) => "user",
                            },
                            onchange: move |evt| {
                                filter_admin.set(match evt.value().as_str() {
                                    "all" => None,
                                    "admin" => Some(true),
                                    "user" => Some(false),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "admin", "仅管理员" }
                            option { value: "user", "仅普通用户" }
                        }
                    }

                    // Deactivated filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "账户状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_deactivated() {
                                None => "all",
                                Some(false) => "active",
                                Some(true) => "deactivated",
                            },
                            onchange: move |evt| {
                                filter_deactivated.set(match evt.value().as_str() {
                                    "all" => None,
                                    "active" => Some(false),
                                    "deactivated" => Some(true),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "active", "活跃" }
                            option { value: "deactivated", "已停用" }
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
                                            class: "block w-full text-left px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                                            onclick: move |_| {
                                                // TODO: Implement batch send notification
                                                show_batch_menu.set(false);
                                            },
                                            "📧 发送服务器通知"
                                        }
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-red-700 hover:bg-red-50",
                                            onclick: move |_| {
                                                // TODO: Implement batch delete
                                                show_batch_menu.set(false);
                                            },
                                            "🗑️ 删除用户"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // User list table
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载用户列表...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if users().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "👥" }
                        p { class: "text-gray-500 text-lg", "暂无用户" }
                        p { class: "text-gray-400 text-sm mt-2", "点击上方"创建用户"按钮添加第一个用户" }
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
                                            checked: !users().is_empty() && selected_users().len() == users().len(),
                                            onchange: move |evt| {
                                                if evt.checked() {
                                                    selected_users.set(users().iter().map(|u| u.user_id.clone()).collect());
                                                } else {
                                                    selected_users.set(Vec::new());
                                                }
                                            }
                                        }
                                    }
                                    th { 
                                        class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100",
                                        onclick: move |_| {
                                            if matches!(sort_by(), UserSortField::Username) {
                                                sort_order.set(match sort_order() {
                                                    SortOrder::Ascending => SortOrder::Descending,
                                                    SortOrder::Descending => SortOrder::Ascending,
                                                });
                                            } else {
                                                sort_by.set(UserSortField::Username);
                                                sort_order.set(SortOrder::Ascending);
                                            }
                                        },
                                        "用户名 {get_sort_indicator(matches!(sort_by(), UserSortField::Username), sort_order())}"
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "显示名" }
                                    th { 
                                        class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100",
                                        onclick: move |_| {
                                            if matches!(sort_by(), UserSortField::CreationTime) {
                                                sort_order.set(match sort_order() {
                                                    SortOrder::Ascending => SortOrder::Descending,
                                                    SortOrder::Descending => SortOrder::Ascending,
                                                });
                                            } else {
                                                sort_by.set(UserSortField::CreationTime);
                                                sort_order.set(SortOrder::Descending);
                                            }
                                        },
                                        "创建时间 {get_sort_indicator(matches!(sort_by(), UserSortField::CreationTime), sort_order())}"
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "状态" }
                                    th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                                }
                            }
                            tbody { class: "bg-white divide-y divide-gray-200",
                                for user in users() {
                                    UserRow {
                                        key: "{user.user_id}",
                                        user: user.clone(),
                                        selected: selected_users().contains(&user.user_id),
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

/// Individual user row component
#[component]
fn UserRow(
    user: User,
    selected: bool,
    on_select: EventHandler<String>,
) -> Element {
    let user_id = user.user_id.clone();
    
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
                        if let Some(avatar_url) = &user.avatar_url {
                            img {
                                class: "h-10 w-10 rounded-full",
                                src: "{avatar_url}",
                                alt: "{user.username}"
                            }
                        } else {
                            div { class: "h-10 w-10 rounded-full bg-gray-300 flex items-center justify-center text-gray-600 font-semibold",
                                "{user.username.chars().next().unwrap_or('U').to_uppercase()}"
                            }
                        }
                    }
                    div { class: "ml-4",
                        div { class: "text-sm font-medium text-gray-900", "{user.username}" }
                        div { class: "text-xs text-gray-500", "{user.user_id}" }
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "text-sm text-gray-900",
                    "{user.display_name.as_ref().unwrap_or(&String::from(\"-\"))}"
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{format_timestamp(user.creation_ts)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "flex gap-2",
                    if user.is_admin {
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-purple-100 text-purple-800",
                            "管理员"
                        }
                    }
                    if user.is_deactivated {
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                            "已停用"
                        }
                    } else {
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                            "活跃"
                        }
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                Link {
                    to: Route::UserDetailPage { user_id: user.user_id.clone() },
                    class: "text-blue-600 hover:text-blue-900",
                    "查看详情"
                }
            }
        }
    }
}

/// Get sort indicator for table headers
fn get_sort_indicator(is_active: bool, order: SortOrder) -> &'static str {
    if !is_active {
        return "↕";
    }
    match order {
        SortOrder::Ascending => "↑",
        SortOrder::Descending => "↓",
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