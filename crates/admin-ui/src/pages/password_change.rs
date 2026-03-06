//! Password Change page component for Web UI admin

use dioxus::prelude::*;
use crate::services::webui_auth_api::WebUIAuthAPI;

/// Password policy validation result
#[derive(Clone, Debug, PartialEq)]
struct PasswordValidation {
    min_length: bool,
    has_uppercase: bool,
    has_lowercase: bool,
    has_digit: bool,
    has_special: bool,
}

impl PasswordValidation {
    fn validate(password: &str) -> Self {
        Self {
            min_length: password.len() >= 12,
            has_uppercase: password.chars().any(|c| c.is_uppercase()),
            has_lowercase: password.chars().any(|c| c.is_lowercase()),
            has_digit: password.chars().any(|c| c.is_numeric()),
            has_special: password.chars().any(|c| !c.is_alphanumeric()),
        }
    }

    fn is_valid(&self) -> bool {
        self.min_length && self.has_uppercase && self.has_lowercase && self.has_digit && self.has_special
    }
}

/// Password Change page component
#[component]
pub fn PasswordChangePage() -> Element {
    let mut current_password = use_signal(|| String::new());
    let mut new_password = use_signal(|| String::new());
    let mut confirm_password = use_signal(|| String::new());
    let mut is_submitting = use_signal(|| false);
    let mut error_message = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);

    let password_validation = use_memo(move || {
        PasswordValidation::validate(&new_password.read())
    });

    let passwords_match = use_memo(move || {
        let new_pwd = new_password.read();
        let confirm = confirm_password.read();
        !new_pwd.is_empty() && !confirm.is_empty() && *new_pwd == *confirm
    });

    let can_submit = use_memo(move || {
        !current_password.read().is_empty() 
            && password_validation.read().is_valid() 
            && *passwords_match.read()
    });

    let handle_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        
        let current = current_password.read().clone();
        let new_pwd = new_password.read().clone();
        let confirm = confirm_password.read().clone();

        // Validate passwords match
        if new_pwd != confirm {
            error_message.set(Some("新密码不匹配".to_string()));
            return;
        }

        // Validate password is different
        if current == new_pwd {
            error_message.set(Some("新密码必须与当前密码不同".to_string()));
            return;
        }

        // Validate password policy
        if !password_validation.read().is_valid() {
            error_message.set(Some("新密码不符合安全策略要求".to_string()));
            return;
        }

        is_submitting.set(true);
        error_message.set(None);
        success_message.set(None);

        spawn(async move {
            match WebUIAuthAPI::change_password(current, new_pwd, confirm).await {
                Ok(response) => {
                    if response.success {
                        success_message.set(Some("密码修改成功！".to_string()));
                        // Clear form
                        current_password.set(String::new());
                        new_password.set(String::new());
                        confirm_password.set(String::new());
                    } else {
                        error_message.set(Some(response.message));
                    }
                    is_submitting.set(false);
                }
                Err(e) => {
                    error_message.set(Some(format!("密码修改失败: {}", e)));
                    is_submitting.set(false);
                }
            }
        });
    };

    rsx! {
        div { class: "min-h-screen bg-gray-50 py-12 px-4 sm:px-6 lg:px-8",
            div { class: "max-w-md mx-auto",
                // Header
                div { class: "mb-8",
                    h2 { class: "text-3xl font-extrabold text-gray-900",
                        "修改密码"
                    }
                    p { class: "mt-2 text-sm text-gray-600",
                        "更新您的 Web UI 管理员密码"
                    }
                }

                // Success message
                if let Some(msg) = success_message.read().as_ref() {
                    div { class: "mb-4 rounded-md bg-green-50 p-4",
                        div { class: "flex",
                            div { class: "ml-3",
                                h3 { class: "text-sm font-medium text-green-800",
                                    "{msg}"
                                }
                            }
                        }
                    }
                }

                // Error message
                if let Some(msg) = error_message.read().as_ref() {
                    div { class: "mb-4 rounded-md bg-red-50 p-4",
                        div { class: "flex",
                            div { class: "ml-3",
                                h3 { class: "text-sm font-medium text-red-800",
                                    "{msg}"
                                }
                            }
                        }
                    }
                }

                // Password change form
                div { class: "bg-white shadow rounded-lg p-6",
                    form {
                        class: "space-y-6",
                        onsubmit: handle_submit,

                        // Current password field
                        div {
                            label {
                                r#for: "current-password",
                                class: "block text-sm font-medium text-gray-700 mb-1",
                                "当前密码"
                            }
                            input {
                                id: "current-password",
                                name: "current-password",
                                r#type: "password",
                                required: true,
                                disabled: *is_submitting.read(),
                                class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                placeholder: "输入当前密码",
                                value: "{current_password}",
                                oninput: move |evt| current_password.set(evt.value().clone())
                            }
                        }

                        // New password field
                        div {
                            label {
                                r#for: "new-password",
                                class: "block text-sm font-medium text-gray-700 mb-1",
                                "新密码"
                            }
                            input {
                                id: "new-password",
                                name: "new-password",
                                r#type: "password",
                                required: true,
                                disabled: *is_submitting.read(),
                                class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                placeholder: "输入新密码",
                                value: "{new_password}",
                                oninput: move |evt| new_password.set(evt.value().clone())
                            }
                            p { class: "mt-1 text-xs text-gray-500",
                                "密码必须至少 12 个字符，包含大写、小写、数字和特殊字符"
                            }
                        }

                        // Confirm new password field
                        div {
                            label {
                                r#for: "confirm-password",
                                class: "block text-sm font-medium text-gray-700 mb-1",
                                "确认新密码"
                            }
                            input {
                                id: "confirm-password",
                                name: "confirm-password",
                                r#type: "password",
                                required: true,
                                disabled: *is_submitting.read(),
                                class: "appearance-none block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm placeholder-gray-400 focus:outline-none focus:ring-blue-500 focus:border-blue-500 sm:text-sm",
                                placeholder: "再次输入新密码",
                                value: "{confirm_password}",
                                oninput: move |evt| confirm_password.set(evt.value().clone())
                            }
                        }

                        // Password policy validation feedback
                        if !new_password.read().is_empty() {
                            div { class: "rounded-md bg-gray-50 p-4",
                                h4 { class: "text-sm font-medium text-gray-900 mb-2",
                                    "密码安全策略"
                                }
                                ul { class: "space-y-1 text-sm",
                                    li {
                                        class: if password_validation.read().min_length { "text-green-600" } else { "text-gray-500" },
                                        span { class: "mr-2",
                                            if password_validation.read().min_length { "✓" } else { "○" }
                                        }
                                        "至少 12 个字符"
                                    }
                                    li {
                                        class: if password_validation.read().has_uppercase { "text-green-600" } else { "text-gray-500" },
                                        span { class: "mr-2",
                                            if password_validation.read().has_uppercase { "✓" } else { "○" }
                                        }
                                        "包含大写字母"
                                    }
                                    li {
                                        class: if password_validation.read().has_lowercase { "text-green-600" } else { "text-gray-500" },
                                        span { class: "mr-2",
                                            if password_validation.read().has_lowercase { "✓" } else { "○" }
                                        }
                                        "包含小写字母"
                                    }
                                    li {
                                        class: if password_validation.read().has_digit { "text-green-600" } else { "text-gray-500" },
                                        span { class: "mr-2",
                                            if password_validation.read().has_digit { "✓" } else { "○" }
                                        }
                                        "包含数字"
                                    }
                                    li {
                                        class: if password_validation.read().has_special { "text-green-600" } else { "text-gray-500" },
                                        span { class: "mr-2",
                                            if password_validation.read().has_special { "✓" } else { "○" }
                                        }
                                        "包含特殊字符"
                                    }
                                }
                            }
                        }

                        // Password match indicator
                        if !new_password.read().is_empty() && !confirm_password.read().is_empty() {
                            div {
                                class: if *passwords_match.read() {
                                    "text-sm text-green-600"
                                } else {
                                    "text-sm text-red-600"
                                },
                                if *passwords_match.read() {
                                    "✓ 密码匹配"
                                } else {
                                    "✗ 密码不匹配"
                                }
                            }
                        }

                        // Submit button
                        div { class: "flex items-center justify-between",
                            button {
                                r#type: "button",
                                disabled: *is_submitting.read(),
                                class: "text-sm font-medium text-gray-600 hover:text-gray-500",
                                onclick: move |_| {
                                    // Navigate back or to dashboard
                                    if let Some(window) = web_sys::window() {
                                        let _ = window.history().and_then(|h| h.back());
                                    }
                                },
                                "返回"
                            }
                            button {
                                r#type: "submit",
                                disabled: *is_submitting.read() || !*can_submit.read(),
                                class: "flex justify-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed",
                                if *is_submitting.read() {
                                    "修改中..."
                                } else {
                                    "修改密码"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
