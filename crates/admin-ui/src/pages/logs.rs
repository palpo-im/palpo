//! Audit logs page component

use dioxus::prelude::*;

/// Audit logs component
#[component]
pub fn AuditLogs() -> Element {
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