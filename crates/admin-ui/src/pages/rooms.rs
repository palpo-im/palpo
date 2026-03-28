//! Room management page component

use dioxus::prelude::*;
use crate::app::Route;
use crate::models::room::{Room, RoomSortField, SortOrder};
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Room manager component with list, search, filter, and batch operations
#[component]
pub fn RoomManager() -> Element {
    // State management
    let rooms = use_signal(|| Vec::<Room>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut search_query = use_signal(|| String::new());
    let mut filter_public = use_signal(|| None::<bool>);
    let mut filter_empty = use_signal(|| None::<bool>);
    let mut sort_by = use_signal(|| RoomSortField::Name);
    let mut sort_order = use_signal(|| SortOrder::Ascending);
    let mut current_page = use_signal(|| 0u32);
    let total_count = use_signal(|| 0u32);
    let mut selected_rooms = use_signal(|| Vec::<String>::new());
    let mut show_batch_menu = use_signal(|| false);
    
    let page_size = 20u32;
    let total_pages = (total_count() + page_size - 1) / page_size;

    // Load rooms effect (placeholder - will be implemented in Phase 3)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 3
        loading.set(false);
    });

    rsx! {
        div { class: "p-4 sm:p-6 space-y-6",
            // Header with actions
            div { class: "flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4",
                div {
                    h2 { class: "text-2xl font-bold text-gray-900", "房间管理" }
                    p { class: "mt-1 text-sm text-gray-500", "管理 Matrix 聊天房间" }
                }
            }

            // Search and filters
            div { class: "bg-white shadow rounded-lg p-4",
                div { class: "grid grid-cols-1 md:grid-cols-4 gap-4",
                    // Search input
                    div { class: "md:col-span-2",
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "搜索房间" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            placeholder: "按房间名称或房间ID搜索...",
                            value: "{search_query}",
                            oninput: move |evt| search_query.set(evt.value())
                        }
                    }

                    // Public room filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "公开状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_public() {
                                None => "all",
                                Some(true) => "public",
                                Some(false) => "private",
                            },
                            onchange: move |evt| {
                                filter_public.set(match evt.value().as_str() {
                                    "all" => None,
                                    "public" => Some(true),
                                    "private" => Some(false),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "public", "仅公开房间" }
                            option { value: "private", "仅私有房间" }
                        }
                    }

                    // Empty room filter
                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-1", "房间状态" }
                        select {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                            value: match filter_empty() {
                                None => "all",
                                Some(true) => "empty",
                                Some(false) => "active",
                            },
                            onchange: move |evt| {
                                filter_empty.set(match evt.value().as_str() {
                                    "all" => None,
                                    "empty" => Some(true),
                                    "active" => Some(false),
                                    _ => None,
                                });
                            },
                            option { value: "all", "全部" }
                            option { value: "active", "有成员" }
                            option { value: "empty", "空房间" }
                        }
                    }
                }
            }

            // Batch operations bar
            if !selected_rooms().is_empty() {
                div { class: "bg-blue-50 border border-blue-200 rounded-lg p-4",
                    div { class: "flex items-center justify-between",
                        div { class: "flex items-center gap-2",
                            span { class: "text-sm font-medium text-blue-900",
                                "已选择 {selected_rooms().len()} 个房间"
                            }
                            button {
                                class: "text-sm text-blue-600 hover:text-blue-800 underline",
                                onclick: move |_| selected_rooms.set(Vec::new()),
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
                                                // TODO: Implement batch publish to directory
                                                show_batch_menu.set(false);
                                            },
                                            "📢 发布到目录"
                                        }
                                        button {
                                            class: "block w-full text-left px-4 py-2 text-sm text-gray-700 hover:bg-gray-100",
                                            onclick: move |_| {
                                                // TODO: Implement batch unpublish from directory
                                                show_batch_menu.set(false);
                                            },
                                            "🚫 从目录取消发布"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Room list table
            div { class: "bg-white shadow rounded-lg overflow-hidden",
                if loading() {
                    div { class: "p-12 text-center",
                        Spinner { size: "large".to_string(), message: Some("加载房间列表...".to_string()) }
                    }
                } else if let Some(err) = error() {
                    div { class: "p-6",
                        ErrorMessage { message: err }
                    }
                } else if rooms().is_empty() {
                    div { class: "p-12 text-center",
                        div { class: "text-gray-400 text-5xl mb-4", "🏠" }
                        p { class: "text-gray-500 text-lg", "暂无房间" }
                        p { class: "text-gray-400 text-sm mt-2", "当前没有找到任何房间" }
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
                                            checked: !rooms().is_empty() && selected_rooms().len() == rooms().len(),
                                            onchange: move |evt| {
                                                if evt.checked() {
                                                    selected_rooms.set(rooms().iter().map(|r| r.room_id.clone()).collect());
                                                } else {
                                                    selected_rooms.set(Vec::new());
                                                }
                                            }
                                        }
                                    }
                                    th { 
                                        class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100",
                                        onclick: move |_| {
                                            if matches!(sort_by(), RoomSortField::Name) {
                                                sort_order.set(match sort_order() {
                                                    SortOrder::Ascending => SortOrder::Descending,
                                                    SortOrder::Descending => SortOrder::Ascending,
                                                });
                                            } else {
                                                sort_by.set(RoomSortField::Name);
                                                sort_order.set(SortOrder::Ascending);
                                            }
                                        },
                                        "房间名称 {get_sort_indicator(matches!(sort_by(), RoomSortField::Name), sort_order())}"
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "房间ID" }
                                    th { 
                                        class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider cursor-pointer hover:bg-gray-100",
                                        onclick: move |_| {
                                            if matches!(sort_by(), RoomSortField::MemberCount) {
                                                sort_order.set(match sort_order() {
                                                    SortOrder::Ascending => SortOrder::Descending,
                                                    SortOrder::Descending => SortOrder::Ascending,
                                                });
                                            } else {
                                                sort_by.set(RoomSortField::MemberCount);
                                                sort_order.set(SortOrder::Descending);
                                            }
                                        },
                                        "成员数 {get_sort_indicator(matches!(sort_by(), RoomSortField::MemberCount), sort_order())}"
                                    }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "加密状态" }
                                    th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "创建者" }
                                    th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                                }
                            }
                            tbody { class: "bg-white divide-y divide-gray-200",
                                for room in rooms() {
                                    RoomRow {
                                        key: "{room.room_id}",
                                        room: room.clone(),
                                        selected: selected_rooms().contains(&room.room_id),
                                        on_select: move |room_id: String| {
                                            let mut current = selected_rooms();
                                            if current.contains(&room_id) {
                                                current.retain(|id| id != &room_id);
                                            } else {
                                                current.push(room_id);
                                            }
                                            selected_rooms.set(current);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Pagination
                    div { class: "px-6 py-4 border-t border-gray-200 flex items-center justify-between",
                        div { class: "text-sm text-gray-700",
                            "显示 {current_page() * page_size + 1} - {((current_page() + 1) * page_size).min(total_count())} / 共 {total_count()} 个房间"
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

/// Individual room row component
#[component]
fn RoomRow(
    room: Room,
    selected: bool,
    on_select: EventHandler<String>,
) -> Element {
    let room_id = room.room_id.clone();
    
    rsx! {
        tr { class: if selected { "bg-blue-50" } else { "hover:bg-gray-50" },
            td { class: "px-6 py-4 whitespace-nowrap",
                input {
                    r#type: "checkbox",
                    class: "rounded border-gray-300 text-blue-600 focus:ring-blue-500",
                    checked: selected,
                    onchange: move |_| on_select.call(room_id.clone())
                }
            }
            td { class: "px-6 py-4",
                div { class: "flex items-center",
                    div { class: "flex-shrink-0 h-10 w-10",
                        if let Some(avatar_url) = &room.avatar_url {
                            img {
                                class: "h-10 w-10 rounded",
                                src: "{avatar_url}",
                                alt: "{room.display_name()}"
                            }
                        } else {
                            div { class: "h-10 w-10 rounded bg-gray-300 flex items-center justify-center text-gray-600 font-semibold",
                                "{room.display_name().chars().next().unwrap_or('R').to_uppercase()}"
                            }
                        }
                    }
                    div { class: "ml-4",
                        div { class: "text-sm font-medium text-gray-900",
                            "{room.display_name()}"
                        }
                        if let Some(topic) = &room.topic {
                            div { class: "text-xs text-gray-500 truncate max-w-xs",
                                "{topic}"
                            }
                        }
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                div { class: "text-xs text-gray-500 font-mono",
                    "{room.room_id}"
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-900",
                "{room.member_count}"
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                if room.is_encrypted {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                        "🔒 已加密"
                    }
                } else {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                        "未加密"
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{room.creator}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                Link {
                    to: Route::RoomDetailPage { room_id: room.room_id.clone() },
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