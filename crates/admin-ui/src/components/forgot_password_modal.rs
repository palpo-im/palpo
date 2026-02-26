//! Forgot password modal component
//!
//! This component displays instructions for resetting the Web UI admin password
//! via direct PostgreSQL database access. Since the Web UI admin is stored in
//! the database with a fixed username "admin", password recovery requires
//! database-level access.

use dioxus::prelude::*;
use crate::components::Button;

/// Forgot password modal component
///
/// Displays a modal dialog with instructions for resetting the Web UI admin password.
/// The modal explains that password recovery requires direct database access and provides
/// SQL command examples for resetting the password.
///
/// # Props
///
/// - `show`: Whether the modal is visible
/// - `onclose`: Event handler called when the modal is closed
///
/// # Requirements
///
/// Implements requirements 4.1-4.7:
/// - 4.1: Display PostgreSQL connection instructions
/// - 4.2: Show SQL command examples
/// - 4.3: Explain database access requirements
/// - 4.4: Provide security warnings
/// - 4.5: Include contact information for support
/// - 4.6: Modal can be dismissed
/// - 4.7: Accessible from login page
#[component]
pub fn ForgotPasswordModal(
    show: bool,
    onclose: EventHandler<()>,
) -> Element {
    if !show {
        return None;
    }

    rsx! {
        // Modal backdrop
        div {
            class: "fixed inset-0 bg-gray-500 bg-opacity-75 transition-opacity z-40",
            onclick: move |_| onclose.call(()),
        }
        
        // Modal dialog
        div {
            class: "fixed inset-0 z-50 overflow-y-auto",
            div {
                class: "flex min-h-full items-end justify-center p-4 text-center sm:items-center sm:p-0",
                div {
                    class: "relative transform overflow-hidden rounded-lg bg-white text-left shadow-xl transition-all sm:my-8 sm:w-full sm:max-w-2xl",
                    onclick: move |evt| evt.stop_propagation(),
                    
                    // Modal header
                    div {
                        class: "bg-white px-4 pt-5 pb-4 sm:p-6 sm:pb-4",
                        div {
                            class: "sm:flex sm:items-start",
                            div {
                                class: "mx-auto flex h-12 w-12 flex-shrink-0 items-center justify-center rounded-full bg-yellow-100 sm:mx-0 sm:h-10 sm:w-10",
                                // Warning icon
                                svg {
                                    class: "h-6 w-6 text-yellow-600",
                                    fill: "none",
                                    view_box: "0 0 24 24",
                                    stroke_width: "1.5",
                                    stroke: "currentColor",
                                    path {
                                        stroke_linecap: "round",
                                        stroke_linejoin: "round",
                                        d: "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"
                                    }
                                }
                            }
                            div {
                                class: "mt-3 text-center sm:mt-0 sm:ml-4 sm:text-left",
                                h3 {
                                    class: "text-lg font-medium leading-6 text-gray-900",
                                    "忘记密码？"
                                }
                                div {
                                    class: "mt-2",
                                    p {
                                        class: "text-sm text-gray-500",
                                        "Web UI 管理员密码存储在 PostgreSQL 数据库中。要重置密码，您需要直接访问数据库。"
                                    }
                                }
                            }
                        }
                    }
                    
                    // Modal content
                    div {
                        class: "px-4 pb-4 sm:px-6 sm:pb-6",
                        
                        // Security warning
                        div {
                            class: "rounded-md bg-yellow-50 p-4 mb-4",
                            div {
                                class: "flex",
                                div {
                                    class: "flex-shrink-0",
                                    svg {
                                        class: "h-5 w-5 text-yellow-400",
                                        view_box: "0 0 20 20",
                                        fill: "currentColor",
                                        path {
                                            fill_rule: "evenodd",
                                            d: "M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 5a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 5zm0 9a1 1 0 100-2 1 1 0 000 2z",
                                            clip_rule: "evenodd"
                                        }
                                    }
                                }
                                div {
                                    class: "ml-3",
                                    h3 {
                                        class: "text-sm font-medium text-yellow-800",
                                        "安全警告"
                                    }
                                    div {
                                        class: "mt-2 text-sm text-yellow-700",
                                        p { "此操作需要数据库管理员权限。请确保您有权访问 PostgreSQL 数据库。" }
                                    }
                                }
                            }
                        }
                        
                        // Instructions
                        div {
                            class: "space-y-4",
                            
                            // Step 1: Connect to database
                            div {
                                h4 {
                                    class: "text-sm font-medium text-gray-900 mb-2",
                                    "步骤 1: 连接到 PostgreSQL 数据库"
                                }
                                div {
                                    class: "bg-gray-50 rounded-md p-3",
                                    code {
                                        class: "text-xs text-gray-800 font-mono",
                                        "psql -U postgres -d palpo"
                                    }
                                }
                                p {
                                    class: "mt-2 text-xs text-gray-500",
                                    "使用您的数据库凭据连接到 Palpo 数据库。"
                                }
                            }
                            
                            // Step 2: Delete existing credential
                            div {
                                h4 {
                                    class: "text-sm font-medium text-gray-900 mb-2",
                                    "步骤 2: 删除现有凭据"
                                }
                                div {
                                    class: "bg-gray-50 rounded-md p-3",
                                    code {
                                        class: "text-xs text-gray-800 font-mono whitespace-pre-wrap",
                                        "DELETE FROM webui_admin_credentials WHERE username = 'admin';"
                                    }
                                }
                                p {
                                    class: "mt-2 text-xs text-gray-500",
                                    "删除现有的管理员凭据记录。"
                                }
                            }
                            
                            // Step 3: Restart and setup
                            div {
                                h4 {
                                    class: "text-sm font-medium text-gray-900 mb-2",
                                    "步骤 3: 重新设置密码"
                                }
                                p {
                                    class: "text-sm text-gray-700",
                                    "刷新此页面，系统将检测到没有管理员账户，并引导您完成设置向导以创建新密码。"
                                }
                            }
                        }
                        
                        // Alternative: Contact support
                        div {
                            class: "mt-6 pt-4 border-t border-gray-200",
                            h4 {
                                class: "text-sm font-medium text-gray-900 mb-2",
                                "需要帮助？"
                            }
                            p {
                                class: "text-sm text-gray-700",
                                "如果您无法访问数据库或需要技术支持，请联系您的系统管理员或数据库管理员。"
                            }
                        }
                    }
                    
                    // Modal footer
                    div {
                        class: "bg-gray-50 px-4 py-3 sm:flex sm:flex-row-reverse sm:px-6",
                        Button {
                            variant: "primary".to_string(),
                            onclick: move |_| onclose.call(()),
                            "我明白了"
                        }
                    }
                }
            }
        }
    }
}
