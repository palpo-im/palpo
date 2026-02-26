//! Main application component

use dioxus::prelude::*;
use crate::models::{AuthState, WebConfigData};
use crate::hooks::use_auth;
use crate::pages::{LoginPage, AdminDashboard, SetupWizardPage, PasswordChangePage};
use crate::services::api_client::init_api_client;
use crate::services::webui_auth_api::WebUIAuthAPI;
use crate::components::layout::AdminLayout as AdminLayoutComponent;

/// Main application routes
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[route("/")]
    Home {},
    #[route("/setup")]
    Setup {},
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
    #[route("/admin/password-change")]
    PasswordChange {},
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
    // Initialize API client on app startup
    use_effect(|| {
        // API server runs on port 8081
        let base_url = "http://localhost:8081".to_string();
        
        init_api_client(base_url);
        
        // Log initialization for debugging
        web_sys::console::log_1(&"API client initialized".into());
    });

    // Initialize global state
    use_context_provider(|| Signal::new(AuthState::Unauthenticated));
    use_context_provider(|| Signal::new(AppState::default()));

    rsx! {
        div { class: "min-h-screen bg-gray-50",
            Router::<Route> {}
        }
    }
}

/// Home page component - redirects to setup, login, or admin based on setup status
#[component]
fn Home() -> Element {
    let auth_context = use_auth();
    let navigator = use_navigator();
    let mut is_checking = use_signal(|| true);
    let mut check_error = use_signal(|| None::<String>);

    use_effect(move || {
        // If already authenticated, go to dashboard
        if auth_context.is_authenticated() {
            navigator.push(Route::Dashboard {});
            return;
        }

        // Check setup status
        spawn(async move {
            match WebUIAuthAPI::get_status().await {
                Ok(status) => {
                    if status.needs_setup {
                        // No admin exists, show setup wizard
                        navigator.push(Route::Setup {});
                    } else {
                        // Admin exists, show login page
                        navigator.push(Route::Login {});
                    }
                }
                Err(e) => {
                    // On error, show error message and default to login
                    web_sys::console::error_1(&format!("Failed to check setup status: {}", e).into());
                    check_error.set(Some(format!("无法检查设置状态: {}", e)));
                    // Default to login page on error
                    navigator.push(Route::Login {});
                }
            }
            is_checking.set(false);
        });
    });

    rsx! {
        div { class: "flex items-center justify-center min-h-screen",
            div { class: "text-center",
                if *is_checking.read() {
                    div { class: "animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600 mx-auto" }
                    p { class: "mt-4 text-gray-600", "正在加载..." }
                } else if let Some(error) = check_error.read().as_ref() {
                    div { class: "text-red-600",
                        p { "{error}" }
                    }
                }
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

/// Setup wizard page component
#[component]
fn Setup() -> Element {
    rsx! {
        SetupWizardPage {}
    }
}

/// Admin layout component with authentication protection
#[component]
fn AdminLayout() -> Element {
    rsx! {
        AdminLayoutComponent {}
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

/// Password change page component
#[component]
fn PasswordChange() -> Element {
    rsx! {
        PasswordChangePage {}
    }
}
