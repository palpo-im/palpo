//! Example usage of layout components
//!
//! This file demonstrates how to use the layout components in a Dioxus application.

#![allow(dead_code)]

use dioxus::prelude::*;
use super::layout::{AdminLayout, Breadcrumb, BreadcrumbItem, Header, NavItem, Sidebar};

/// Example: Basic admin layout usage
#[component]
pub fn BasicLayoutExample() -> Element {
    rsx! {
        AdminLayout {}
    }
}

/// Example: Custom sidebar with navigation items
#[component]
pub fn CustomSidebarExample() -> Element {
    let mut show_mobile = use_signal(|| false);

    rsx! {
        div { class: "flex h-screen",
            Sidebar {
                show_mobile: show_mobile(),
                on_close: move |_| show_mobile.set(false)
            }
            
            main { class: "flex-1",
                button {
                    class: "m-4 px-4 py-2 bg-blue-600 text-white rounded",
                    onclick: move |_| show_mobile.set(true),
                    "Toggle Mobile Menu"
                }
                
                div { class: "p-6",
                    h1 { "Main Content Area" }
                    p { "This is where your page content goes." }
                }
            }
        }
    }
}

/// Example: Header with breadcrumbs
#[component]
pub fn HeaderWithBreadcrumbsExample() -> Element {
    let mut show_menu = use_signal(|| false);

    rsx! {
        div { class: "min-h-screen bg-gray-50",
            Header {
                on_menu_toggle: move |_| show_menu.set(!show_menu())
            }
            
            div { class: "p-6",
                h2 { "Page Content" }
                p { "The header above includes breadcrumb navigation." }
            }
        }
    }
}

/// Example: Standalone breadcrumb component
#[component]
pub fn BreadcrumbExample() -> Element {
    let breadcrumbs = vec![
        BreadcrumbItem {
            label: "首页".to_string(),
            route: Some("/admin".to_string()),
        },
        BreadcrumbItem {
            label: "配置管理".to_string(),
            route: Some("/admin/config".to_string()),
        },
        BreadcrumbItem {
            label: "服务器配置".to_string(),
            route: None, // Current page
        },
    ];

    rsx! {
        div { class: "p-6 bg-white",
            h2 { class: "text-xl font-semibold mb-4", "Breadcrumb Navigation" }
            Breadcrumb { items: breadcrumbs }
            
            div { class: "mt-6",
                p { "The breadcrumb shows the navigation hierarchy." }
                p { class: "text-sm text-gray-500 mt-2",
                    "Current page items are not clickable, while parent items are links."
                }
            }
        }
    }
}

/// Example: Responsive layout demonstration
#[component]
pub fn ResponsiveLayoutExample() -> Element {
    let mut show_mobile = use_signal(|| false);

    rsx! {
        div { class: "flex h-screen bg-gray-50",
            // Sidebar - hidden on mobile, visible on desktop
            Sidebar {
                show_mobile: show_mobile(),
                on_close: move |_| show_mobile.set(false)
            }
            
            // Main content
            main { class: "flex-1 flex flex-col",
                Header {
                    on_menu_toggle: move |_| show_mobile.set(!show_mobile())
                }
                
                div { class: "flex-1 overflow-auto p-6",
                    div { class: "max-w-4xl mx-auto",
                        h1 { class: "text-3xl font-bold mb-4", "Responsive Layout" }
                        
                        div { class: "bg-white rounded-lg shadow p-6 mb-6",
                            h2 { class: "text-xl font-semibold mb-2", "Desktop View (lg+)" }
                            ul { class: "list-disc list-inside space-y-1 text-gray-700",
                                li { "Sidebar always visible" }
                                li { "No mobile menu button" }
                                li { "Full breadcrumb navigation" }
                                li { "Extended session info" }
                            }
                        }
                        
                        div { class: "bg-white rounded-lg shadow p-6",
                            h2 { class: "text-xl font-semibold mb-2", "Mobile View (<lg)" }
                            ul { class: "list-disc list-inside space-y-1 text-gray-700",
                                li { "Sidebar hidden by default" }
                                li { "Mobile menu button visible" }
                                li { "Sidebar slides in from left" }
                                li { "Backdrop overlay when open" }
                                li { "Compact header layout" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Example: Navigation items configuration
pub fn example_nav_items() -> Vec<NavItem> {
    vec![
        NavItem {
            id: "dashboard",
            label: "仪表板",
            icon: "📊",
            route: "/admin".to_string(),
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
    ]
}

/// Example: Breadcrumb generation helper
pub fn generate_breadcrumbs(path: &str) -> Vec<BreadcrumbItem> {
    let mut breadcrumbs = vec![
        BreadcrumbItem {
            label: "首页".to_string(),
            route: Some("/admin".to_string()),
        }
    ];

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    
    for (i, segment) in segments.iter().enumerate() {
        if i == 0 && *segment == "admin" {
            continue; // Skip "admin" as it's the home
        }
        
        let label = match *segment {
            "config" => "配置管理",
            "users" => "用户管理",
            "rooms" => "房间管理",
            "federation" => "联邦管理",
            "media" => "媒体管理",
            "appservices" => "应用服务",
            "logs" => "审计日志",
            _ => segment,
        };
        
        let is_last = i == segments.len() - 1;
        breadcrumbs.push(BreadcrumbItem {
            label: label.to_string(),
            route: if is_last { None } else { Some(format!("/{}", segments[..=i].join("/"))) },
        });
    }

    breadcrumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nav_items_generation() {
        let items = example_nav_items();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].id, "dashboard");
        assert_eq!(items[0].label, "仪表板");
    }

    #[test]
    fn test_breadcrumb_generation() {
        let breadcrumbs = generate_breadcrumbs("/admin/config");
        assert_eq!(breadcrumbs.len(), 2);
        assert_eq!(breadcrumbs[0].label, "首页");
        assert_eq!(breadcrumbs[1].label, "配置管理");
        assert!(breadcrumbs[1].route.is_none()); // Current page
    }

    #[test]
    fn test_breadcrumb_generation_nested() {
        let breadcrumbs = generate_breadcrumbs("/admin/users/details");
        assert_eq!(breadcrumbs.len(), 3);
        assert_eq!(breadcrumbs[0].label, "首页");
        assert_eq!(breadcrumbs[1].label, "用户管理");
        assert!(breadcrumbs[1].route.is_some()); // Parent page
        assert_eq!(breadcrumbs[2].label, "details");
        assert!(breadcrumbs[2].route.is_none()); // Current page
    }
}
