//! Setup Wizard page component for initial Web UI admin account creation

use dioxus::prelude::*;
use crate::services::webui_auth_api::WebUIAuthAPI;
use web_sys::window;

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

/// Check if legacy credentials exist in localStorage
fn check_legacy_credentials() -> bool {
    if let Some(window) = window() {
        if let Ok(Some(storage)) = window.local_storage() {
            if let Ok(Some(_)) = storage.get_item("palpo_web_ui_admin") {
                return true;
            }
        }
    }
    false
}

/// Setup Wizard page component
#[component]
pub fn SetupWizardPage() -> Element {
    let mut password = use_signal(|| String::new());
    let mut confirm_password = use_signal(|| String::new());
    let mut show_migration = use_signal(|| false);
    let mut is_submitting = use_signal(|| false);
    let mut error_message = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);
    let mut show_migration_option = use_signal(|| false);

    // Check for legacy credentials on mount
    use_effect(move || {
        let has_legacy = check_legacy_credentials();
        show_migration_option.set(has_legacy);
    });

    let password_validation = use_memo(move || {
        PasswordValidation::validate(&password.read())
    });

    let passwords_match = use_memo(move || {
        let pwd = password.read();
        let confirm = confirm_password.read();
        !pwd.is_empty() && !confirm.is_empty() && *pwd == *confirm
    });

    let can_submit = use_memo(move || {
        password_validation.read().is_valid() && *passwords_match.read()
    });

    let handle_setup = move |_| {
        let pwd = password.read().clone();
        let confirm = confirm_password.read().clone();

        if pwd != confirm {
            error_message.set(Some("密码不匹配".to_string()));
            return;
        }

        if !password_validation.read().is_valid() {
            error_message.set(Some("密码不符合安全策略要求".to_string()));
            return;
        }

        is_submitting.set(true);
        error_message.set(None);

        spawn(async move {
            match WebUIAuthAPI::setup(pwd).await {
                Ok(response) => {
                    if response.success {
                        success_message.set(Some("管理员账号创建成功！正在跳转到登录页面...".to_string()));
                        
                        // Redirect to login page immediately
                        if let Some(window) = window() {
                            let _ = window.location().set_href("/login");
                        }
                    } else {
                        error_message.set(Some(response.message));
                        is_submitting.set(false);
                    }
                }
                Err(e) => {
                    error_message.set(Some(format!("创建失败: {}", e)));
                    is_submitting.set(false);
                }
            }
        });
    };

    let handle_migration = move |_| {
        let pwd = password.read().clone();

        if pwd.is_empty() {
            error_message.set(Some("请输入当前密码以进行迁移".to_string()));
            return;
        }

        is_submitting.set(true);
        error_message.set(None);

        spawn(async move {
            match WebUIAuthAPI::migrate(pwd).await {
                Ok(response) => {
                    if response.success {
                        success_message.set(Some("凭据迁移成功！正在跳转到登录页面...".to_string()));
                        
                        // Redirect to login page immediately
                        if let Some(window) = window() {
                            let _ = window.location().set_href("/login");
                        }
                    } else {
                        error_message.set(Some(response.message));
                        is_submitting.set(false);
                    }
                }
                Err(e) => {
                    error_message.set(Some(format!("迁移失败: {}", e)));
                    is_submitting.set(false);
                }
            }
        });
    };

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8",
            div { class: "max-w-md w-full space-y-8",
                // Header
                div {
                    div { class: "mx-auto h-12 w-12 bg-blue-600 rounded-lg flex items-center justify-center",
                        span { class: "text-white font-bold text-xl", "P" }
                    }
                    h2 { class: "mt-6 text-center text-3xl font-extrabold text-gray-900",
                        "欢迎使用 Palpo 管理界面"
                    }
                    p { class: "mt-2 text-center text-sm text-gray-600",
                        "首次使用，请设置管理员密码"
                    }
                }

                // Migration option if legacy credentials detected
                if *show_migration_option.read() && !*show_migration.read() {
                    div { class: "rounded-md bg-blue-50 p-4",
                        div { class: "flex",
                            div { class: "ml-3",
                                h3 { class: "text-sm font-medium text-blue-800",
                                    "检测到旧版凭据"
                                }
                                div { class: "mt-2 text-sm text-blue-700",
                                    p { "我们检测到您之前使用的凭据存储在浏览器中。您可以选择：" }
                                    div { class: "mt-2 space-x-2",
                                        button {
                                            r#type: "button",
                                            class: "text-sm font-medium text-blue-600 hover:text-blue-500",
                                            onclick: move |_| show_migration.set(true),
                                            "迁移现有凭据"
                                        }
                                        span { class: "text-gray-400", "或" }
                                        button {
                                            r#type: "button",
                                            class: "text-sm font-medium text-blue-600 hover:text-blue-500",
                                            onclick: move |_| show_migration_option.set(false),
                                            "创建新凭据"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Success message
                if let Some(msg) = success_message.read().as_ref() {
                    div { class: "rounded-md bg-green-50 p-4",
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
                    div { class: "rounded-md bg-red-50 p-4",
                        div { class: "flex",
                            div { class: "ml-3",
                                h3 { class: "text-sm font-medium text-red-800",
                                    "{msg}"
                                }
                            }
                        }
                    }
                }

                // Migration form
                if *show_migration.read() {
                    form {
                        class: "mt-8 space-y-6",
                        onsubmit: handle_migration,
                        prevent_default: "onsubmit",

                        div { class: "rounded-md shadow-sm",
                            div {
                                label {
                                    r#for: "migration-password",
                                    class: "block text-sm font-medium text-gray-700 mb-1",
                                    "当前密码"
                                }
                                input {
                                    id: "migration-password",
                                    name: "password",
                                    r#type: "password",
                                    required: true,
                                    disabled: *is_submitting.read(),
                                    class: "appearance-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm",
                                    placeholder: "输入您的当前密码",
                                    value: "{password}",
                                    oninput: move |evt| password.set(evt.value().clone())
                                }
                            }
                        }

                        div { class: "flex items-center justify-between",
                            button {
                                r#type: "button",
                                disabled: *is_submitting.read(),
                                class: "text-sm font-medium text-gray-600 hover:text-gray-500",
                                onclick: move |_| {
                                    show_migration.set(false);
                                    password.set(String::new());
                                },
                                "返回"
                            }
                            button {
                                r#type: "submit",
                                disabled: *is_submitting.read() || password.read().is_empty(),
                                class: "group relative flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed",
                                if *is_submitting.read() {
                                    "迁移中..."
                                } else {
                                    "迁移凭据"
                                }
                            }
                        }
                    }
                } else {
                    // Setup form
                    form {
                        class: "mt-8 space-y-6",
                        onsubmit: handle_setup,
                        prevent_default: "onsubmit",

                        div { class: "rounded-md shadow-sm space-y-4",
                            // Username field (fixed as "admin")
                            div {
                                label {
                                    r#for: "username",
                                    class: "block text-sm font-medium text-gray-700 mb-1",
                                    "用户名"
                                }
                                input {
                                    id: "username",
                                    name: "username",
                                    r#type: "text",
                                    disabled: true,
                                    class: "appearance-none relative block w-full px-3 py-2 border border-gray-300 bg-gray-100 text-gray-500 rounded-md sm:text-sm",
                                    value: "admin"
                                }
                                p { class: "mt-1 text-xs text-gray-500",
                                    "用户名固定为 admin"
                                }
                            }

                            // Password field
                            div {
                                label {
                                    r#for: "password",
                                    class: "block text-sm font-medium text-gray-700 mb-1",
                                    "密码"
                                }
                                input {
                                    id: "password",
                                    name: "password",
                                    r#type: "password",
                                    required: true,
                                    disabled: *is_submitting.read(),
                                    class: "appearance-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm",
                                    placeholder: "输入密码",
                                    value: "{password}",
                                    oninput: move |evt| password.set(evt.value().clone())
                                }
                            }

                            // Confirm password field
                            div {
                                label {
                                    r#for: "confirm-password",
                                    class: "block text-sm font-medium text-gray-700 mb-1",
                                    "确认密码"
                                }
                                input {
                                    id: "confirm-password",
                                    name: "confirm-password",
                                    r#type: "password",
                                    required: true,
                                    disabled: *is_submitting.read(),
                                    class: "appearance-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm",
                                    placeholder: "再次输入密码",
                                    value: "{confirm_password}",
                                    oninput: move |evt| confirm_password.set(evt.value().clone())
                                }
                            }
                        }

                        // Password policy validation feedback
                        if !password.read().is_empty() {
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
                        if !password.read().is_empty() && !confirm_password.read().is_empty() {
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
                        div {
                            button {
                                r#type: "submit",
                                disabled: *is_submitting.read() || !*can_submit.read(),
                                class: "group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed",
                                if *is_submitting.read() {
                                    "创建中..."
                                } else {
                                    "创建管理员账号"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
