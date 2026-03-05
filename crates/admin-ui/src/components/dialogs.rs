//! Action dialogs for user management operations

use dioxus::prelude::*;
use crate::components::forms::Button;

/// Dialog type enumeration
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DialogType {
    Deactivate,
    DeleteDevice,
    DeleteMedia,
    ShadowBan,
    LoginAsUser,
    PasswordReset,
}

/// Props for the ConfirmationDialog component
#[derive(Props, Clone, PartialEq)]
pub struct ConfirmationDialogProps {
    /// Dialog type
    pub dialog_type: DialogType,
    /// Title of the dialog
    pub title: String,
    /// Message to display
    pub message: String,
    /// Item being acted on (user_id, device_id, etc.)
    pub target: String,
    /// Whether the dialog is visible
    pub visible: bool,
    /// Whether the action is loading
    pub loading: bool,
    /// Callback when confirmed
    pub on_confirm: EventHandler<()>,
    /// Callback when cancelled
    pub on_cancel: EventHandler<()>,
}

/// Confirmation dialog component for destructive actions
#[component]
pub fn ConfirmationDialog(props: ConfirmationDialogProps) -> Element {
    if !props.visible {
        return None;
    }

    let (icon, button_variant, button_text) = match props.dialog_type {
        DialogType::Deactivate => ("❌", "red", "确认停用"),
        DialogType::DeleteDevice => ("🗑️", "red", "确认删除"),
        DialogType::DeleteMedia => ("🗑️", "red", "确认删除"),
        DialogType::ShadowBan => ("⚠️", "yellow", "确认暗封"),
        DialogType::LoginAsUser => ("⚠️", "yellow", "确认登录"),
        DialogType::PasswordReset => ("🔑", "blue", "确认重置"),
    };

    rsx! {
        div { class: "modal-overlay",
            div { class: "modal-content max-w-md",
                div { class: "p-6",
                    // Header
                    div { class: "flex items-center gap-3 mb-4",
                        span { class: "text-3xl", "{icon}" }
                        h3 { class: "text-lg font-medium text-gray-900", "{props.title}" }
                    }

                    // Message
                    div { class: "mb-6",
                        p { class: "text-gray-600", "{props.message}" }
                        div { class: "mt-3 p-3 bg-gray-100 rounded text-sm font-mono text-gray-800 break-all",
                            "{props.target}"
                        }
                    }

                    // Warning for dangerous actions
                    if matches!(props.dialog_type, DialogType::Deactivate | DialogType::ShadowBan | DialogType::LoginAsUser) {
                        div { class: "mb-6 p-4 bg-yellow-50 border border-yellow-200 rounded-md",
                            p { class: "text-sm text-yellow-800 font-medium", "⚠️ 请注意" }
                            ul { class: "mt-2 text-sm text-yellow-700 list-disc list-inside",
                                if props.dialog_type == DialogType::Deactivate {
                                    li { "用户将被停用，无法登录" }
                                    li { "可以选择是否删除用户数据" }
                                    li { "此操作可以撤销" }
                                } else if props.dialog_type == DialogType::ShadowBan {
                                    li { "用户不会知道自己被封禁" }
                                    li { "用户的操作会看似成功但实际失败" }
                                    li { "谨慎使用此功能" }
                                } else if props.dialog_type == DialogType::LoginAsUser {
                                    li { "将以该用户身份登录" }
                                    li { "所有操作将记录为该用户" }
                                    li { "请确保有正当理由" }
                                }
                            }
                        }
                    }

                    // Actions
                    div { class: "flex justify-end gap-3",
                        Button {
                            variant: "secondary".to_string(),
                            onclick: move |_| props.on_cancel.call(()),
                            disabled: props.loading,
                            "取消"
                        }
                        Button {
                            variant: button_variant.to_string(),
                            onclick: move |_| props.on_confirm.call(()),
                            loading: props.loading,
                            "{button_text}"
                        }
                    }
                }
            }
        }
    }
}

/// Props for the PasswordResetDialog component
#[derive(Props, Clone, PartialEq)]
pub struct PasswordResetDialogProps {
    /// User ID
    pub user_id: String,
    /// Whether the dialog is visible
    pub visible: bool,
    /// Whether the action is loading
    pub loading: bool,
    /// Generated password (if any)
    pub generated_password: Option<String>,
    /// Callback when confirmed
    pub on_confirm: EventHandler<bool>, // logout_devices
    /// Callback when cancelled
    pub on_cancel: EventHandler<()>,
}

/// Password reset dialog component
#[component]
pub fn PasswordResetDialog(props: PasswordResetDialogProps) -> Element {
    let mut logout_devices = use_signal(|| true);

    if !props.visible {
        return None;
    }

    rsx! {
        div { class: "modal-overlay",
            div { class: "modal-content max-w-md",
                div { class: "p-6",
                    // Header
                    div { class: "flex items-center gap-3 mb-4",
                        span { class: "text-3xl", "🔑" }
                        h3 { class: "text-lg font-medium text-gray-900", "重置密码" }
                    }

                    // Message
                    div { class: "mb-6",
                        p { class: "text-gray-600", "将为以下用户重置密码：" }
                        div { class: "mt-2 p-3 bg-gray-100 rounded text-sm font-mono text-gray-800 break-all",
                            "{props.user_id}"
                        }
                    }

                    // Options
                    div { class: "mb-6 space-y-3",
                        label { class: "flex items-center gap-2",
                            input {
                                r#type: "checkbox",
                                class: "h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded",
                                checked: logout_devices(),
                                onchange: move |evt| logout_devices.set(evt.checked())
                            }
                            span { class: "text-sm text-gray-700", "使该用户的所有设备退出登录" }
                        }
                    }

                    // Generated password display
                    if let Some(pwd) = &props.generated_password {
                        div { class: "mb-6 p-4 bg-green-50 border border-green-200 rounded-md",
                            p { class: "text-sm text-green-800 font-medium", "新密码：" }
                            div { class: "mt-2 p-3 bg-white rounded font-mono text-lg text-center tracking-wider",
                                "{pwd}"
                            }
                            p { class: "mt-2 text-xs text-green-600", "请立即复制并妥善保管此密码" }
                        }
                    }

                    // Actions
                    div { class: "flex justify-end gap-3",
                        Button {
                            variant: "secondary".to_string(),
                            onclick: move |_| props.on_cancel.call(()),
                            disabled: props.loading,
                            "取消"
                        }
                        Button {
                            variant: "blue".to_string(),
                            onclick: move |_| props.on_confirm.call(logout_devices()),
                            loading: props.loading,
                            "重置密码"
                        }
                    }
                }
            }
        }
    }
}

/// Props for the SuccessDialog component
#[derive(Props, Clone, PartialEq)]
pub struct SuccessDialogProps {
    /// Title
    pub title: String,
    /// Message
    pub message: String,
    /// Icon emoji
    #[props(default = "✅".to_string())]
    pub icon: String,
    /// Whether the dialog is visible
    pub visible: bool,
    /// Callback when closed
    pub on_close: EventHandler<()>,
}

/// Success dialog component
#[component]
pub fn SuccessDialog(props: SuccessDialogProps) -> Element {
    if !props.visible {
        return None;
    }

    rsx! {
        div { class: "modal-overlay",
            div { class: "modal-content max-w-md",
                div { class: "p-6 text-center",
                    // Icon
                    div { class: "mb-4",
                        span { class: "text-5xl", "{props.icon}" }
                    }

                    // Title
                    h3 { class: "text-lg font-medium text-gray-900 mb-2", "{props.title}" }

                    // Message
                    p { class: "text-gray-600 mb-6", "{props.message}" }

                    // Close button
                    Button {
                        variant: "primary".to_string(),
                        onclick: move |_| props.on_close.call(()),
                        "确定"
                    }
                }
            }
        }
    }
}