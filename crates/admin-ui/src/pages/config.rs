//! Configuration management page component

use dioxus::prelude::*;

/// Configuration manager component
#[component]
pub fn ConfigManager() -> Element {
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