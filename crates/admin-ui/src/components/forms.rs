//! Form components for the admin UI

use dioxus::prelude::*;
use rand::Rng;
use super::feedback::ErrorMessage;

/// Props for the Input component
#[derive(Props, Clone, PartialEq)]
pub struct InputProps {
    /// Input label
    pub label: String,
    /// Input type (text, password, email, etc.)
    #[props(default = "text".to_string())]
    pub input_type: String,
    /// Current value
    pub value: String,
    /// Placeholder text
    #[props(default = String::new())]
    pub placeholder: String,
    /// Whether the input is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the input is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub oninput: EventHandler<String>,
}

/// Text input component with label and validation
#[component]
pub fn Input(props: InputProps) -> Element {
    let has_error = props.error.is_some();
    let input_class = if has_error {
        "form-input error"
    } else {
        "form-input"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            input {
                r#type: "{props.input_type}",
                class: "{input_class}",
                value: "{props.value}",
                placeholder: "{props.placeholder}",
                readonly: props.readonly,
                required: props.required,
                oninput: move |evt| props.oninput.call(evt.value().clone())
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the TextArea component
#[derive(Props, Clone, PartialEq)]
pub struct TextAreaProps {
    /// TextArea label
    pub label: String,
    /// Current value
    pub value: String,
    /// Placeholder text
    #[props(default = String::new())]
    pub placeholder: String,
    /// Number of rows
    #[props(default = 4.0)]
    pub rows: f64,
    /// Whether the textarea is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the textarea is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub oninput: EventHandler<String>,
}

/// TextArea component with label and validation
#[component]
pub fn TextArea(props: TextAreaProps) -> Element {
    let has_error = props.error.is_some();
    let textarea_class = if has_error {
        "form-textarea error"
    } else {
        "form-textarea"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            textarea {
                class: "{textarea_class}",
                value: "{props.value}",
                placeholder: "{props.placeholder}",
                rows: props.rows,
                readonly: props.readonly,
                required: props.required,
                oninput: move |evt| props.oninput.call(evt.value().clone())
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Select component
#[derive(Props, Clone, PartialEq)]
pub struct SelectProps {
    /// Select label
    pub label: String,
    /// Current selected value
    pub value: String,
    /// Available options (value, label, description)
    #[props(default = vec![])]
    pub options: Vec<(String, String, Option<String>)>,
    /// Whether the select is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Whether the select is required
    #[props(default = false)]
    pub required: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when value changes
    pub onchange: EventHandler<String>,
}

/// Select dropdown component with label and validation
#[component]
pub fn Select(props: SelectProps) -> Element {
    let has_error = props.error.is_some();
    let select_class = if has_error {
        "form-select error"
    } else {
        "form-select"
    };

    rsx! {
        div { class: "form-group",
            label { class: "form-label",
                "{props.label}"
                if props.required {
                    span { class: "required", " *" }
                }
            }
            select {
                class: "{select_class}",
                value: "{props.value}",
                disabled: props.readonly,
                required: props.required,
                onchange: move |evt| props.onchange.call(evt.value().clone()),
                for (value, label, description) in props.options.iter() {
                    option { value: "{value}", "{label}" }
                }
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Checkbox component
#[derive(Props, Clone, PartialEq)]
pub struct CheckboxProps {
    /// Checkbox label
    pub label: String,
    /// Current checked state
    pub checked: bool,
    /// Whether the checkbox is readonly
    #[props(default = false)]
    pub readonly: bool,
    /// Error message to display
    #[props(default = None)]
    pub error: Option<String>,
    /// Callback when checked state changes
    pub onchange: EventHandler<bool>,
}

/// Checkbox component with label and validation
#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    let has_error = props.error.is_some();
    let checkbox_class = if has_error {
        "form-checkbox error"
    } else {
        "form-checkbox"
    };

    rsx! {
        div { class: "form-group checkbox-group",
            label { class: "checkbox-label",
                input {
                    r#type: "checkbox",
                    class: "{checkbox_class}",
                    checked: props.checked,
                    disabled: props.readonly,
                    onchange: move |evt| props.onchange.call(evt.checked())
                }
                span { "{props.label}" }
            }
            if let Some(error) = &props.error {
                ErrorMessage { message: error.clone() }
            }
        }
    }
}

/// Props for the Button component
#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    /// Button text
    pub children: Element,
    /// Button variant (primary, secondary, danger, success)
    #[props(default = "primary".to_string())]
    pub variant: String,
    /// Button size (small, medium, large)
    #[props(default = "medium".to_string())]
    pub size: String,
    /// Whether the button is disabled
    #[props(default = false)]
    pub disabled: bool,
    /// Whether the button is in loading state
    #[props(default = false)]
    pub loading: bool,
    /// Button type (button, submit, reset)
    #[props(default = "button".to_string())]
    pub button_type: String,
    /// Click handler
    #[props(default = EventHandler::default())]
    pub onclick: EventHandler<MouseEvent>,
}

/// Button component with variants and loading state
#[component]
pub fn Button(props: ButtonProps) -> Element {
    let button_class = format!("btn btn-{} btn-{}", props.variant, props.size);
    let is_disabled = props.disabled || props.loading;

    rsx! {
        button {
            r#type: "{props.button_type}",
            class: "{button_class}",
            disabled: is_disabled,
            onclick: move |evt| {
                if !is_disabled {
                    props.onclick.call(evt)
                }
            },
            if props.loading {
                span { class: "btn-spinner" }
            }
            {props.children}
        }
    }
}

// ===== User Form Component =====

use crate::models::user::{CreateUserRequest, User};

/// Props for the UserForm component
#[derive(Props, Clone, PartialEq)]
pub struct UserFormProps {
    /// Existing user to edit (None for create mode)
    #[props(default = None)]
    pub user: Option<User>,
    /// Callback when form is submitted successfully
    pub on_success: EventHandler<User>,
    /// Callback when form is cancelled
    pub on_cancel: EventHandler<()>,
}

/// User creation/edition form component
#[component]
pub fn UserForm(props: UserFormProps) -> Element {
    let is_edit = props.user.is_some();
    let mut is_loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    // Form state
    let mut username = use_signal(|| props.user.as_ref().map(|u| u.username.clone()).unwrap_or_default());
    let mut display_name = use_signal(|| props.user.as_ref().and_then(|u| u.display_name.clone()).unwrap_or_default());
    let mut avatar_url = use_signal(|| props.user.as_ref().and_then(|u| u.avatar_url.clone()).unwrap_or_default());
    let mut is_admin = use_signal(|| props.user.as_ref().map(|u| u.is_admin).unwrap_or(false));
    let mut password = use_signal(|| String::new());
    let mut confirm_password = use_signal(|| String::new());
    let mut generated_password = use_signal(|| None::<String>);

    // Validation state
    let mut username_error = use_signal(|| None::<String>);
    let mut password_error = use_signal(|| None::<String>);
    let mut confirm_error = use_signal(|| None::<String>);

    // Validate username
    let mut validate_username = move || -> bool {
        let username_val = username();
        if username_val.is_empty() {
            username_error.set(Some("用户名不能为空".to_string()));
            return false;
        }
        if username_val.len() < 3 {
            username_error.set(Some("用户名至少3个字符".to_string()));
            return false;
        }
        if username_val.len() > 255 {
            username_error.set(Some("用户名不能超过255个字符".to_string()));
            return false;
        }
        if !username_val.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            username_error.set(Some("用户名只能包含字母、数字、下划线和连字符".to_string()));
            return false;
        }
        username_error.set(None);
        true
    };

    // Validate password
    let mut validate_password = move || -> bool {
        if !is_edit && password().is_empty() {
            password_error.set(Some("密码不能为空".to_string()));
            return false;
        }
        if !password().is_empty() && password().len() < 8 {
            password_error.set(Some("密码至少8个字符".to_string()));
            return false;
        }
        password_error.set(None);
        true
    };

    // Validate confirm password
    let mut validate_confirm = move || -> bool {
        if password() != confirm_password() {
            confirm_error.set(Some("两次输入的密码不一致".to_string()));
            return false;
        }
        confirm_error.set(None);
        true
    };

    // Generate random password
    let mut generate_random_password = move || {
        let chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*";
        let mut rng = rand::thread_rng();
        let new_password: String = (0..16)
            .map(|_| chars.chars().nth(rng.gen_range(0..chars.len())).unwrap())
            .collect();
        generated_password.set(Some(new_password.clone()));
        password.set(new_password);
    };

    // Handle form submission
    let mut handle_submit = move |_| {
        let username_valid = validate_username();
        let password_valid = validate_password();
        let confirm_valid = validate_confirm();

        if !username_valid || !password_valid || !confirm_valid {
            return;
        }

        is_loading.set(true);
        error.set(None);

        let request = CreateUserRequest {
            username: username(),
            password: if password().is_empty() { None } else { Some(password()) },
            display_name: if display_name().is_empty() { None } else { Some(display_name()) },
            is_admin: is_admin(),
            permissions: vec![],
            send_notification: false,
        };

        // Spawn async task to create user
        let on_success = props.on_success.clone();
        let mut error_clone = error.clone();
        let mut is_loading_clone = is_loading.clone();

        // Use wasm_bindgen_futures::spawn_local for WASM compatibility
        wasm_bindgen_futures::spawn_local(async move {
            // Use global API client
            let api = match crate::services::api_client::get_api_client() {
                Ok(client) => crate::services::user_admin_api::UserAdminAPI::new(
                    crate::utils::audit_logger::AuditLogger::new(1000),
                    client,
                ),
                Err(e) => {
                    error_clone.set(Some(e.to_string()));
                    is_loading_clone.set(false);
                    return;
                }
            };

            match api.create_user(request, "admin").await {
                Ok(response) => {
                    is_loading_clone.set(false);
                    if response.success {
                        if let Some(user) = response.user {
                            on_success.call(user);
                        }
                    } else {
                        error_clone.set(response.error.or(Some("创建用户失败".to_string())));
                    }
                }
                Err(e) => {
                    is_loading_clone.set(false);
                    error_clone.set(Some(e.to_string()));
                }
            }
        });
    };

    rsx! {
        div { class: "modal-overlay",
            div { class: "modal-content",
                div { class: "modal-header",
                    h3 { class: "text-lg font-medium text-gray-900",
                        if is_edit {
                            "编辑用户"
                        } else {
                            "创建用户"
                        }
                    }
                    button {
                        class: "modal-close",
                        onclick: move |_| props.on_cancel.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    if let Some(err) = error() {
                        div { class: "mb-4 p-4 bg-red-50 border border-red-200 rounded-md",
                            p { class: "text-sm text-red-600", "{err}" }
                        }
                    }

                    form {
                        onsubmit: move |e| {
                            e.prevent_default();
                            handle_submit(());
                        },
                        // Username field
                        div { class: "mb-4",
                            label { class: "block text-sm font-medium text-gray-700 mb-1", "用户名 *" }
                            input {
                                r#type: "text",
                                class: format!("w-full px-3 py-2 border rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 {}",
                                    if username_error().is_some() { "border-red-300" } else { "border-gray-300" }),
                                value: "{username}",
                                placeholder: "输入用户名",
                                disabled: is_edit,
                                oninput: move |evt| {
                                    username.set(evt.value().clone());
                                    validate_username();
                                }
                            }
                            if let Some(err) = username_error() {
                                p { class: "mt-1 text-sm text-red-600", "{err}" }
                            }
                        }

                        // Display name field
                        div { class: "mb-4",
                            label { class: "block text-sm font-medium text-gray-700 mb-1", "显示名" }
                            input {
                                r#type: "text",
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                value: "{display_name}",
                                placeholder: "输入显示名（可选）",
                                oninput: move |evt| display_name.set(evt.value().clone())
                            }
                        }

                        // Avatar URL field
                        div { class: "mb-4",
                            label { class: "block text-sm font-medium text-gray-700 mb-1", "头像 URL" }
                            input {
                                r#type: "text",
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                value: "{avatar_url}",
                                placeholder: "https://example.com/avatar.jpg（可选）",
                                oninput: move |evt| avatar_url.set(evt.value().clone())
                            }
                        }

                        // Password field (create mode only)
                        if !is_edit {
                            div { class: "mb-4",
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "密码" }
                                div { class: "flex gap-2",
                                    input {
                                        r#type: "password",
                                        class: format!("flex-1 px-3 py-2 border rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 {}",
                                            if password_error().is_some() { "border-red-300" } else { "border-gray-300" }),
                                        value: "{password}",
                                        placeholder: "输入密码",
                                        oninput: move |evt| {
                                            password.set(evt.value().clone());
                                            validate_password();
                                        }
                                    }
                                    button {
                                        type: "button",
                                        class: "px-4 py-2 bg-gray-100 border border-gray-300 rounded-md text-sm font-medium text-gray-700 hover:bg-gray-200",
                                        onclick: move |_| generate_random_password(),
                                        "🎲 生成"
                                    }
                                }
                                if let Some(err) = password_error() {
                                    p { class: "mt-1 text-sm text-red-600", "{err}" }
                                }
                                if let Some(gen_pwd) = generated_password() {
                                    div { class: "mt-2 p-2 bg-blue-50 border border-blue-200 rounded text-sm",
                                        p { class: "text-blue-800 font-medium", "生成的密码: {gen_pwd}" }
                                        p { class: "text-blue-600 text-xs mt-1", "请妥善保管此密码" }
                                    }
                                }
                            }

                            // Confirm password field
                            div { class: "mb-4",
                                label { class: "block text-sm font-medium text-gray-700 mb-1", "确认密码 *" }
                                input {
                                    r#type: "password",
                                    class: format!("w-full px-3 py-2 border rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 {}",
                                        if confirm_error().is_some() { "border-red-300" } else { "border-gray-300" }),
                                    value: "{confirm_password}",
                                    placeholder: "再次输入密码",
                                    oninput: move |evt| {
                                        confirm_password.set(evt.value().clone());
                                        validate_confirm();
                                    }
                                }
                                if let Some(err) = confirm_error() {
                                    p { class: "mt-1 text-sm text-red-600", "{err}" }
                                }
                            }
                        }

                        // Admin checkbox
                        div { class: "mb-6",
                            label { class: "flex items-center",
                                input {
                                    r#type: "checkbox",
                                    class: "h-4 w-4 text-blue-600 focus:ring-blue-500 border-gray-300 rounded",
                                    checked: is_admin(),
                                    onchange: move |evt| is_admin.set(evt.checked())
                                }
                                span { class: "ml-2 text-sm text-gray-700", "授予管理员权限" }
                            }
                        }

                        // Form actions
                        div { class: "flex justify-end gap-3",
                            button {
                                type: "button",
                                class: "px-4 py-2 border border-gray-300 rounded-md text-sm font-medium text-gray-700 bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500",
                                onclick: move |_| props.on_cancel.call(()),
                                "取消"
                            }
                            button {
                                type: "submit",
                                class: format!("px-4 py-2 border border-transparent text-sm font-medium rounded-md text-white {}",
                                    if is_loading() { "bg-blue-400 cursor-not-allowed" } else { "bg-blue-600 hover:bg-blue-700" }),
                                disabled: is_loading(),
                                if is_loading() {
                                    "创建中..."
                                } else {
                                    if is_edit {
                                        "保存更改"
                                    } else {
                                        "创建用户"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

