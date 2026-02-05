//! Main application component

use dioxus::prelude::*;
use dioxus_router::prelude::*;
use crate::models::{AuthState, WebConfigData};
use crate::hooks::use_auth;
use crate::pages::{LoginPage, AdminDashboard};

/// Main application routes
#[derive(Clone, Routable, Debug, PartialEq)]
enum Route {
    #[route("/")]
    Home {},
    #[route("/login")]
    Login {},
    #[layout(AdminLayout)]
    #[route("/admin")]
    Dashboard {},
    #[route("/admin/config")]
    Config {},
    #[route("/admin/users")]
    Users {},
    #[route("/admin/rooms")]
    Rooms {},
    #[route("/admin/federation")]
    Federation {},
    #[route("/admin/media")]
    Media {},
    #[route("/admin/appservices")]
    Appservices {},
    #[route("/admin/logs")]
    Logs {},
}

/// Global application state
#[derive(Clone, Debug, PartialEq)]
pub struct AppState {
    pub config: Option<WebConfigData>,
    pub is_loading: bool,
    pub error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            config: None,
            is_loading: false,
            error: None,
        }
    }
}

/// Main application component with routing and state management
#[component]
pub fn App() -> Element {
    // Initialize global state
    use_context_provider(|| Signal::new(AuthState::Unauthenticated));
    use_context_provider(|| Signal::new(AppState::default()));

    rsx! {
        div { class: "min-h-screen bg-gray-50",
            Router::<Route> {}
        }
    }
}

/// Home page component - redirects to admin or login
#[component]
fn Home() -> Element {
    let auth_context = use_auth();
    let navigator = use_navigator();

    use_effect(move || {
        if auth_context.is_authenticated() {
            navigator.push(Route::Dashboard {});
        } else {
            navigator.push(Route::Login {});
        }
    });

    rsx! {
        div { class: "flex items-center justify-center min-h-screen",
            div { class: "text-center",
                div { class: "animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600 mx-auto" }
                p { class: "mt-4 text-gray-600", "正在加载..." }
            }
        }
    }
}

/// Login page component
#[component]
fn Login() -> Element {
    let auth_context = use_auth();
    let navigator = use_navigator();

    // Redirect if already authenticated
    use_effect({
        let auth_context = auth_context.clone();
        let navigator = navigator.clone();
        move || {
            if auth_context.is_authenticated() {
                navigator.push(Route::Dashboard {});
            }
        }
    });

    rsx! {
        LoginPage {}
    }
}

/// Admin layout component with authentication protection
#[component]
fn AdminLayout() -> Element {
    let auth_context = use_auth();
    let navigator = use_navigator();

    // Check authentication and redirect if needed
    use_effect({
        let auth_context = auth_context.clone();
        let navigator = navigator.clone();
        move || {
            if !auth_context.is_authenticated() {
                navigator.push(Route::Login {});
            }
        }
    });

    // Don't render admin content if not authenticated
    if !auth_context.is_authenticated() {
        return rsx! {
            div { class: "flex items-center justify-center min-h-screen",
                div { class: "text-center",
                    div { class: "animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600 mx-auto" }
                    p { class: "mt-4 text-gray-600", "验证身份中..." }
                }
            }
        };
    }

    rsx! {
        div { class: "flex h-screen bg-gray-50",
            // Sidebar navigation
            AdminSidebar {}
            
            // Main content area
            main { class: "flex-1 overflow-hidden",
                div { class: "flex flex-col h-full",
                    // Header
                    AdminHeader {}
                    
                    // Content
                    div { class: "flex-1 overflow-auto p-6",
                        Outlet::<Route> {}
                    }
                }
            }
        }
    }
}

/// Admin sidebar navigation component
#[component]
fn AdminSidebar() -> Element {
    let auth_context = use_auth();
    let current_route = use_route::<Route>();

    let nav_items = vec![
        ("dashboard", "仪表板", Route::Dashboard {}),
        ("config", "配置管理", Route::Config {}),
        ("users", "用户管理", Route::Users {}),
        ("rooms", "房间管理", Route::Rooms {}),
        ("federation", "联邦管理", Route::Federation {}),
        ("media", "媒体管理", Route::Media {}),
        ("appservices", "应用服务", Route::Appservices {}),
        ("logs", "审计日志", Route::Logs {}),
    ];

    rsx! {
        aside { class: "w-64 bg-white shadow-lg",
            div { class: "flex flex-col h-full",
                // Logo and title
                div { class: "flex items-center px-6 py-4 border-b",
                    div { class: "flex items-center",
                        div { class: "w-8 h-8 bg-blue-600 rounded-lg flex items-center justify-center",
                            span { class: "text-white font-bold text-sm", "P" }
                        }
                        span { class: "ml-3 text-xl font-semibold text-gray-900", "Palpo 管理" }
                    }
                }
                
                // Navigation menu
                nav { class: "flex-1 px-4 py-6 space-y-2",
                    for (_icon, label, route) in nav_items {
                        Link {
                            to: route.clone(),
                            class: format!(
                                "flex items-center px-4 py-2 text-sm font-medium rounded-lg transition-colors {}",
                                if current_route == route {
                                    "bg-blue-100 text-blue-700"
                                } else {
                                    "text-gray-600 hover:bg-gray-100 hover:text-gray-900"
                                }
                            ),
                            span { class: "mr-3", "📊" } // Using emoji for now, can be replaced with proper icons
                            span { "{label}" }
                        }
                    }
                }
                
                // User info and logout
                div { class: "px-4 py-4 border-t",
                    if let Some(user) = auth_context.current_user() {
                        div { class: "flex items-center",
                            div { class: "flex-1",
                                p { class: "text-sm font-medium text-gray-900", "{user.username}" }
                                p { class: "text-xs text-gray-500", "管理员" }
                            }
                            button {
                                onclick: move |_| auth_context.logout(),
                                class: "ml-3 text-sm text-gray-500 hover:text-gray-700",
                                "退出"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Admin header component
#[component]
fn AdminHeader() -> Element {
    let auth_context = use_auth();
    let current_route = use_route::<Route>();

    let page_title = match current_route {
        Route::Dashboard {} => "仪表板",
        Route::Config {} => "配置管理",
        Route::Users {} => "用户管理",
        Route::Rooms {} => "房间管理",
        Route::Federation {} => "联邦管理",
        Route::Media {} => "媒体管理",
        Route::Appservices {} => "应用服务管理",
        Route::Logs {} => "审计日志",
        _ => "管理界面",
    };

    rsx! {
        header { class: "bg-white shadow-sm border-b px-6 py-4",
            div { class: "flex items-center justify-between",
                div {
                    h1 { class: "text-2xl font-semibold text-gray-900", "{page_title}" }
                }
                
                div { class: "flex items-center space-x-4",
                    // Session info
                    if let Some(user) = auth_context.current_user() {
                        div { class: "text-sm text-gray-500",
                            if let Some(remaining) = user.remaining_session_time() {
                                span { "会话剩余: {remaining / 60}分钟" }
                            } else {
                                span { class: "text-red-500", "会话已过期" }
                            }
                        }
                    }
                    
                    // Logout button
                    button {
                        onclick: move |_| auth_context.logout(),
                        class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-blue-500",
                        "退出登录"
                    }
                }
            }
        }
    }
}

/// Dashboard page component
#[component]
fn Dashboard() -> Element {
    rsx! {
        AdminDashboard {}
    }
}

/// Config manager page component
#[component]
fn Config() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "配置管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Palpo Matrix 服务器配置"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "配置管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// User manager page component
#[component]
fn Users() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "用户管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Matrix 用户账户"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "用户管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// Room manager page component
#[component]
fn Rooms() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "房间管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Matrix 聊天房间"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "房间管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// Federation manager page component
#[component]
fn Federation() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "联邦管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Matrix 联邦连接"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "联邦管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// Media manager page component
#[component]
fn Media() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "媒体管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理媒体文件和存储"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "媒体管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// Appservice manager page component
#[component]
fn Appservices() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "应用服务管理"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "管理 Matrix 应用服务"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "应用服务管理功能正在开发中..." }
                    }
                }
            }
        }
    }
}

/// Audit logs page component
#[component]
fn Logs() -> Element {
    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900",
                        "审计日志"
                    }
                    p { class: "mt-1 text-sm text-gray-500",
                        "查看系统操作审计日志"
                    }
                    div { class: "mt-8 text-center py-12",
                        p { class: "text-gray-500", "审计日志功能正在开发中..." }
                    }
                }
            }
        }
    }
}