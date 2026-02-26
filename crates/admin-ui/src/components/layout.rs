//! Layout components for admin interface
//!
//! This module provides the core layout components including:
//! - AdminLayout: Main layout wrapper with sidebar and header
//! - Sidebar: Responsive navigation sidebar
//! - Header: Page header with breadcrumbs and user info
//! - Breadcrumb: Navigation breadcrumb component

use dioxus::prelude::*;
use crate::hooks::use_auth;

/// Navigation item definition
#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub route: String,
}

/// Breadcrumb item definition
#[derive(Clone, Debug, PartialEq)]
pub struct BreadcrumbItem {
    pub label: String,
    pub route: Option<String>,
}

/// Admin layout component with sidebar and header
///
/// This component provides the main layout structure for admin pages,
/// including authentication protection, responsive sidebar, and header.
#[component]
pub fn AdminLayout() -> Element {
    let auth_context = use_auth();
    let mut show_mobile_menu = use_signal(|| false);

    // Check authentication
    if !auth_context.is_authenticated() {
        return rsx! {
            div { class: "flex items-center justify-center min-h-screen bg-gray-50",
                div { class: "text-center",
                    div { class: "animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600 mx-auto" }
                    p { class: "mt-4 text-gray-600", "验证身份中..." }
                }
            }
        };
    }

    rsx! {
        div { class: "flex h-screen bg-gray-50 overflow-hidden",
            // Sidebar
            Sidebar {
                show_mobile: show_mobile_menu(),
                on_close: move |_| show_mobile_menu.set(false)
            }
            
            // Main content area
            main { class: "flex-1 flex flex-col overflow-hidden",
                // Header with breadcrumbs
                Header {
                    on_menu_toggle: move |_| show_mobile_menu.set(!show_mobile_menu())
                }
                
                // Page content
                div { class: "flex-1 overflow-auto",
                    div { class: "container mx-auto px-4 sm:px-6 lg:px-8 py-6",
                        Outlet::<Route> {}
                    }
                }
            }
        }
    }
}

/// Responsive sidebar navigation component
#[component]
pub fn Sidebar(
    show_mobile: bool,
    on_close: EventHandler<()>,
) -> Element {
    let auth_context = use_auth();
    let route = use_route::<Route>();

    let nav_items = vec![
        NavItem {
            id: "dashboard",
            label: "仪表板",
            icon: "📊",
            route: "/admin".to_string(),
        },
        NavItem {
            id: "server-control",
            label: "服务器控制",
            icon: "🎛️",
            route: "/admin/server-control".to_string(),
        },
        NavItem {
            id: "config",
            label: "配置管理",
            icon: "⚙️",
            route: "/admin/config".to_string(),
        },
        NavItem {
            id: "users",
            label: "用户管理",
            icon: "👥",
            route: "/admin/users".to_string(),
        },
        NavItem {
            id: "rooms",
            label: "房间管理",
            icon: "🏠",
            route: "/admin/rooms".to_string(),
        },
        NavItem {
            id: "federation",
            label: "联邦管理",
            icon: "🌐",
            route: "/admin/federation".to_string(),
        },
        NavItem {
            id: "media",
            label: "媒体管理",
            icon: "🖼️",
            route: "/admin/media".to_string(),
        },
        NavItem {
            id: "appservices",
            label: "应用服务",
            icon: "🔌",
            route: "/admin/appservices".to_string(),
        },
        NavItem {
            id: "logs",
            label: "审计日志",
            icon: "📝",
            route: "/admin/logs".to_string(),
        },
    ];

    let current_path = route.to_string();
    let is_active = move |item_route: &str| current_path.starts_with(item_route);

    rsx! {
        // Mobile overlay
        if show_mobile {
            div {
                class: "fixed inset-0 z-40 lg:hidden",
                onclick: move |_| on_close.call(()),
                div { class: "fixed inset-0 bg-gray-600 bg-opacity-75" }
            }
        }

        // Sidebar
        aside {
            class: format!(
                "fixed inset-y-0 left-0 z-50 w-64 bg-white shadow-lg transform transition-transform duration-300 ease-in-out lg:translate-x-0 lg:static lg:inset-0 {}",
                if show_mobile { "translate-x-0" } else { "-translate-x-full" }
            ),
            
            div { class: "flex flex-col h-full",
                // Logo and title
                div { class: "flex items-center justify-between px-6 py-4 border-b",
                    div { class: "flex items-center",
                        div { class: "w-10 h-10 bg-gradient-to-br from-blue-600 to-blue-700 rounded-lg flex items-center justify-center shadow-md",
                            span { class: "text-white font-bold text-lg", "P" }
                        }
                        span { class: "ml-3 text-xl font-semibold text-gray-900", "Palpo 管理" }
                    }
                    
                    // Close button for mobile
                    button {
                        class: "lg:hidden p-2 rounded-md text-gray-400 hover:text-gray-500 hover:bg-gray-100",
                        onclick: move |_| on_close.call(()),
                        "✕"
                    }
                }
                
                // Navigation menu
                nav { class: "flex-1 px-3 py-4 space-y-1 overflow-y-auto",
                    for item in nav_items {
                        Link {
                            key: "{item.id}",
                            to: item.route.clone(),
                            class: format!(
                                "flex items-center px-3 py-2.5 text-sm font-medium rounded-lg transition-all duration-150 {}",
                                if is_active(&item.route) {
                                    "bg-blue-50 text-blue-700 shadow-sm"
                                } else {
                                    "text-gray-700 hover:bg-gray-50 hover:text-gray-900"
                                }
                            ),
                            onclick: move |_| {
                                // Close mobile menu when navigating
                                on_close.call(());
                            },
                            
                            span { class: "text-lg mr-3", "{item.icon}" }
                            span { "{item.label}" }
                            
                            if is_active(&item.route) {
                                span { class: "ml-auto w-1.5 h-1.5 bg-blue-600 rounded-full" }
                            }
                        }
                    }
                }
                
                // User info and logout
                div { class: "px-4 py-4 border-t bg-gray-50",
                    if let Some(user) = auth_context.current_user() {
                        div { class: "flex items-center",
                            div { class: "flex-shrink-0",
                                div { class: "w-10 h-10 bg-gradient-to-br from-gray-400 to-gray-500 rounded-full flex items-center justify-center text-white font-semibold",
                                    "{user.username.chars().next().unwrap_or('U').to_uppercase()}"
                                }
                            }
                            div { class: "ml-3 flex-1 min-w-0",
                                p { class: "text-sm font-medium text-gray-900 truncate", "{user.username}" }
                                p { class: "text-xs text-gray-500", "管理员" }
                            }
                            button {
                                onclick: move |_| auth_context.logout(),
                                class: "ml-2 p-2 text-gray-400 hover:text-gray-600 rounded-md hover:bg-gray-100 transition-colors",
                                title: "退出登录",
                                "🚪"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Header component with breadcrumbs and user actions
#[component]
pub fn Header(on_menu_toggle: EventHandler<()>) -> Element {
    let auth_context = use_auth();
    let route = use_route::<Route>();

    // Generate breadcrumbs based on current route
    let breadcrumbs = get_breadcrumbs(&route);
    let page_title = breadcrumbs.last().map(|b| b.label.clone()).unwrap_or_default();

    rsx! {
        header { class: "bg-white shadow-sm border-b",
            div { class: "px-4 sm:px-6 lg:px-8 py-4",
                div { class: "flex items-center justify-between",
                    // Left side: Menu button and breadcrumbs
                    div { class: "flex items-center flex-1 min-w-0",
                        // Mobile menu button
                        button {
                            class: "lg:hidden p-2 mr-2 rounded-md text-gray-400 hover:text-gray-500 hover:bg-gray-100",
                            onclick: move |_| on_menu_toggle.call(()),
                            "☰"
                        }
                        
                        div { class: "flex-1 min-w-0",
                            // Page title
                            h1 { class: "text-xl sm:text-2xl font-semibold text-gray-900 truncate",
                                "{page_title}"
                            }
                            
                            // Breadcrumbs
                            Breadcrumb { items: breadcrumbs }
                        }
                    }
                    
                    // Right side: User actions
                    div { class: "flex items-center space-x-3 sm:space-x-4",
                        // Session info
                        if let Some(user) = auth_context.current_user() {
                            div { class: "hidden sm:block text-sm text-gray-500",
                                if let Some(remaining) = user.remaining_session_time() {
                                    span { 
                                        class: if remaining < 300 { "text-orange-600 font-medium" } else { "" },
                                        "会话: {remaining / 60}分钟"
                                    }
                                } else {
                                    span { class: "text-red-600 font-medium", "会话已过期" }
                                }
                            }
                        }
                        
                        // Logout button
                        button {
                            onclick: move |_| auth_context.logout(),
                            class: "inline-flex items-center px-3 py-2 sm:px-4 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 transition-colors",
                            span { class: "hidden sm:inline", "退出登录" }
                            span { class: "sm:hidden", "退出" }
                        }
                    }
                }
            }
        }
    }
}

/// Breadcrumb navigation component
#[component]
pub fn Breadcrumb(items: Vec<BreadcrumbItem>) -> Element {
    if items.len() <= 1 {
        return rsx! { div {} };
    }

    rsx! {
        nav { class: "flex mt-1", "aria-label": "Breadcrumb",
            ol { class: "flex items-center space-x-2 text-sm",
                for (index, item) in items.iter().enumerate() {
                    li {
                        key: "{index}",
                        class: "flex items-center",
                        
                        if index > 0 {
                            span { class: "mx-2 text-gray-400", "/" }
                        }
                        
                        if let Some(route) = &item.route {
                            Link {
                                to: route.clone(),
                                class: "text-gray-500 hover:text-gray-700 transition-colors",
                                "{item.label}"
                            }
                        } else {
                            span { class: "text-gray-900 font-medium", "{item.label}" }
                        }
                    }
                }
            }
        }
    }
}

/// Generate breadcrumbs based on current route
fn get_breadcrumbs(route: &Route) -> Vec<BreadcrumbItem> {
    let mut breadcrumbs = vec![
        BreadcrumbItem {
            label: "首页".to_string(),
            route: Some("/admin".to_string()),
        }
    ];

    match route {
        Route::Dashboard {} => {
            breadcrumbs.last_mut().unwrap().route = None;
        }
        Route::ServerControl {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "服务器控制".to_string(),
                route: None,
            });
        }
        Route::Config {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "配置管理".to_string(),
                route: None,
            });
        }
        Route::Users {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "用户管理".to_string(),
                route: None,
            });
        }
        Route::Rooms {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "房间管理".to_string(),
                route: None,
            });
        }
        Route::Federation {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "联邦管理".to_string(),
                route: None,
            });
        }
        Route::Media {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "媒体管理".to_string(),
                route: None,
            });
        }
        Route::Appservices {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "应用服务".to_string(),
                route: None,
            });
        }
        Route::Logs {} => {
            breadcrumbs.push(BreadcrumbItem {
                label: "审计日志".to_string(),
                route: None,
            });
        }
        _ => {}
    }

    breadcrumbs
}

// Re-export Route for convenience
pub use crate::app::Route;
