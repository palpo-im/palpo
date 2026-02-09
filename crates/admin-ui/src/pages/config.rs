//! Configuration management page component
//!
//! This module provides a comprehensive web interface for managing Palpo Matrix server configuration.
//! It implements a tabbed interface with 7 configuration sections, real-time validation,
//! and save/reset functionality.
//!
//! # Features
//!
//! - **Grouped Configuration Sections**: Server, Database, Federation, Auth, Media, Network, Logging
//! - **Real-time Validation**: Field-level validation with immediate error feedback
//! - **Dirty State Tracking**: Enables save/reset buttons only when changes are made
//! - **Success/Error Feedback**: Clear user feedback for all operations
//! - **Responsive Layout**: Sidebar navigation with dynamic content area
//!
//! # Architecture
//!
//! ```text
//! ConfigManager (Main Component)
//! ├── Header (Title + Save/Reset Buttons)
//! ├── SectionNavigation (Sidebar with 7 tabs)
//! └── SectionContent (Dynamic form based on active section)
//!     ├── ServerConfigForm
//!     ├── DatabaseConfigForm
//!     ├── FederationConfigForm
//!     ├── AuthConfigForm
//!     ├── MediaConfigForm
//!     ├── NetworkConfigForm
//!     └── LoggingConfigForm
//! ```

use dioxus::prelude::*;
use crate::components::forms::{Input, Select, Checkbox, Button};
use crate::components::feedback::{ErrorMessage, SuccessMessage};
use crate::components::loading::Spinner;
use crate::hooks::use_config::use_config_loader;
use crate::models::config::*;
use std::collections::HashMap;

/// Main configuration manager component
///
/// This component provides a complete interface for managing all Palpo server configuration.
/// It uses Dioxus signals for reactive state management and integrates with the configuration
/// API for loading, validating, and saving configuration changes.
///
/// # State Management
///
/// - `form_data`: Current configuration being edited
/// - `validation_errors`: Map of field names to error messages
/// - `is_dirty`: Whether unsaved changes exist
/// - `save_success`: Whether the last save was successful
/// - `active_section`: Currently displayed configuration section
///
/// # User Flow
///
/// 1. User opens page → Configuration loads automatically
/// 2. User selects a section → Form displays for that section
/// 3. User modifies fields → `is_dirty` becomes true, save button enables
/// 4. User clicks save → Validation runs, then saves if valid
/// 5. User clicks reset → Reverts to original configuration
#[component]
pub fn ConfigManager() -> Element {
    // Load configuration context with API methods
    let config_context = use_config_loader();
    
    // Local reactive state for form management
    let mut form_data = use_signal(|| config_context.current_config().unwrap_or_default());
    let mut validation_errors = use_signal(|| HashMap::<String, String>::new());
    let mut is_dirty = use_signal(|| false);
    let mut save_success = use_signal(|| false);
    let mut active_section = use_signal(|| "server".to_string());
    
    // Load configuration when component mounts
    {
        let config_context = config_context.clone();
        use_effect(move || {
            if let Some(config) = config_context.current_config() {
                form_data.set(config);
            }
        });
    }
    
    // Handle save button click
    // Validates configuration before saving to prevent invalid configs
    let handle_save = {
        let config_context = config_context.clone();
        move |_| {
            let config = form_data.read().clone();
            
            // Validate configuration before saving
            config_context.validate_config(config.clone());
            
            // Only save if validation passes
            if config_context.error().is_none() {
                config_context.save_config(config);
                is_dirty.set(false);
                save_success.set(true);
            }
        }
    };
    
    // Handle reset button click
    // Reverts all changes to the original loaded configuration
    let handle_reset = {
        let config_context = config_context.clone();
        move |_| {
            if let Some(original_config) = config_context.current_config() {
                form_data.set(original_config);
                is_dirty.set(false);
                save_success.set(false);
                validation_errors.set(HashMap::new());
            }
        }
    };
    
    let is_loading = config_context.is_loading();
    let error = config_context.error();
    
    let element: Element = rsx! {
        div { class: "space-y-6",
            // Header section with title and action buttons
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "flex justify-between items-center",
                        div {
                            h3 { class: "text-lg leading-6 font-medium text-gray-900",
                                "配置管理"
                            }
                            p { class: "mt-1 text-sm text-gray-500",
                                "管理 Palpo Matrix 服务器配置"
                            }
                        }
                        div { class: "flex space-x-3",
                            Button {
                                variant: "secondary".to_string(),
                                disabled: !is_dirty(),
                                onclick: handle_reset,
                                "重置"
                            }
                            Button {
                                variant: "primary".to_string(),
                                disabled: !is_dirty() || is_loading,
                                loading: is_loading,
                                onclick: handle_save,
                                "保存配置"
                            }
                        }
                    }
                    
                    // Success/Error messages
                    if save_success() {
                        div { class: "mt-4",
                            SuccessMessage { message: "配置已成功保存".to_string() }
                        }
                    }
                    if let Some(err) = error {
                        div { class: "mt-4",
                            ErrorMessage { message: err }
                        }
                    }
                }
            }
            
            // Configuration form
            if is_loading {
                div { class: "bg-white shadow rounded-lg p-12",
                    div { class: "flex justify-center",
                        Spinner { size: "large".to_string() }
                    }
                }
            } else {
                div { class: "bg-white shadow rounded-lg",
                    div { class: "flex",
                        // Section navigation
                        div { class: "w-64 border-r border-gray-200",
                            nav { class: "space-y-1 p-4",
                                SectionNavItem {
                                    label: "服务器配置".to_string(),
                                    section: "server".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("server".to_string())
                                }
                                SectionNavItem {
                                    label: "数据库配置".to_string(),
                                    section: "database".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("database".to_string())
                                }
                                SectionNavItem {
                                    label: "联邦配置".to_string(),
                                    section: "federation".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("federation".to_string())
                                }
                                SectionNavItem {
                                    label: "认证配置".to_string(),
                                    section: "auth".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("auth".to_string())
                                }
                                SectionNavItem {
                                    label: "媒体配置".to_string(),
                                    section: "media".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("media".to_string())
                                }
                                SectionNavItem {
                                    label: "网络配置".to_string(),
                                    section: "network".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("network".to_string())
                                }
                                SectionNavItem {
                                    label: "日志配置".to_string(),
                                    section: "logging".to_string(),
                                    active_section: active_section(),
                                    onclick: move |_| active_section.set("logging".to_string())
                                }
                            }
                        }
                        
                        // Section content
                        div { class: "flex-1 p-6",
                            match active_section().as_str() {
                                "server" => rsx! {
                                    ServerConfigForm {
                                        config: form_data().server.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "database" => rsx! {
                                    DatabaseConfigForm {
                                        config: form_data().database.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "federation" => rsx! {
                                    FederationConfigForm {
                                        config: form_data().federation.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "auth" => rsx! {
                                    AuthConfigForm {
                                        config: form_data().auth.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "media" => rsx! {
                                    MediaConfigForm {
                                        config: form_data().media.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "network" => rsx! {
                                    NetworkConfigForm {
                                        config: form_data().network.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                "logging" => rsx! {
                                    LoggingConfigForm {
                                        config: form_data().logging.clone(),
                                        errors: validation_errors(),
                                        form_data: form_data,
                                        is_dirty: is_dirty,
                                        save_success: save_success,
                                        validation_errors: validation_errors
                                    }
                                },
                                _ => rsx! { div { "未知配置节" } }
                            }
                        }
                    }
                }
            }
        }
    };
    
    element
}

/// Section navigation item component
///
/// Renders a single navigation button in the sidebar for switching between configuration sections.
/// Highlights the active section and handles click events to change the active section.
///
/// # Props
///
/// - `label`: Display text for the navigation item
/// - `section`: Internal identifier for the section (e.g., "server", "database")
/// - `active_section`: Currently active section identifier
/// - `onclick`: Event handler for when the item is clicked
#[component]
fn SectionNavItem(
    label: String,
    section: String,
    active_section: String,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let is_active = section == active_section;
    let class = if is_active {
        "block px-3 py-2 rounded-md text-sm font-medium bg-blue-50 text-blue-700"
    } else {
        "block px-3 py-2 rounded-md text-sm font-medium text-gray-700 hover:bg-gray-50 hover:text-gray-900"
    };
    
    rsx! {
        button {
            class: "{class}",
            onclick: move |evt| onclick.call(evt),
            "{label}"
        }
    }
}

/// Server configuration form component
///
/// Displays and manages server-related configuration fields including:
/// - Server name (Matrix server domain)
/// - Maximum request size
/// - Metrics monitoring toggle
///
/// # Props
///
/// - `config`: Current server configuration values
/// - `errors`: Validation errors map (field name -> error message)
/// - `form_data`: Mutable signal to the complete configuration
/// - `is_dirty`: Mutable signal tracking unsaved changes
/// - `save_success`: Mutable signal for save success state
/// - `validation_errors`: Mutable signal for validation errors
///
/// # Behavior
///
/// When a field is modified:
/// 1. Sets `is_dirty` to true (enables save button)
/// 2. Clears `save_success` flag
/// 3. Updates the field in `form_data`
/// 4. Removes any existing validation error for that field
#[component]
fn ServerConfigForm(
    config: ServerConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "服务器配置" }
            
            Input {
                label: "服务器名称".to_string(),
                value: config.server_name.clone(),
                required: true,
                error: errors.get("server.server_name").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.server.server_name = val.clone());
                    validation_errors.with_mut(|errs| { errs.remove("server.server_name"); });
                }
            }
            
            Input {
                label: "最大请求大小 (字节)".to_string(),
                input_type: "number".to_string(),
                value: config.max_request_size.to_string(),
                required: true,
                error: errors.get("server.max_request_size").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(size) = val.parse() {
                        form_data.with_mut(|cfg| cfg.server.max_request_size = size);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("server.max_request_size"); });
                }
            }
            
            Checkbox {
                label: "启用指标监控".to_string(),
                checked: config.enable_metrics,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.server.enable_metrics = checked);
                }
            }
        }
    }
}

/// Database configuration form component
///
/// Manages PostgreSQL database connection settings including connection string,
/// pool configuration, and migration options.
#[component]
fn DatabaseConfigForm(
    config: DatabaseConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "数据库配置" }
            
            Input {
                label: "数据库连接字符串".to_string(),
                value: config.connection_string.clone(),
                required: true,
                error: errors.get("database.connection_string").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.database.connection_string = val.clone());
                    validation_errors.with_mut(|errs| { errs.remove("database.connection_string"); });
                }
            }
            
            Input {
                label: "最大连接数".to_string(),
                input_type: "number".to_string(),
                value: config.max_connections.to_string(),
                required: true,
                error: errors.get("database.max_connections").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(num) = val.parse() {
                        form_data.with_mut(|cfg| cfg.database.max_connections = num);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("database.max_connections"); });
                }
            }
            
            Input {
                label: "连接超时 (秒)".to_string(),
                input_type: "number".to_string(),
                value: config.connection_timeout.to_string(),
                required: true,
                error: errors.get("database.connection_timeout").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(timeout) = val.parse() {
                        form_data.with_mut(|cfg| cfg.database.connection_timeout = timeout);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("database.connection_timeout"); });
                }
            }
            
            Checkbox {
                label: "自动迁移数据库".to_string(),
                checked: config.auto_migrate,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.database.auto_migrate = checked);
                }
            }
        }
    }
}

/// Federation configuration form component
///
/// Controls Matrix federation settings including enable/disable toggle,
/// signing key configuration, and key verification options.
#[component]
fn FederationConfigForm(
    config: FederationConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "联邦配置" }
            
            Checkbox {
                label: "启用联邦功能".to_string(),
                checked: config.enabled,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.federation.enabled = checked);
                }
            }
            
            Input {
                label: "签名密钥路径".to_string(),
                value: config.signing_key_path.clone(),
                required: true,
                error: errors.get("federation.signing_key_path").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.federation.signing_key_path = val.clone());
                    validation_errors.with_mut(|errs| { errs.remove("federation.signing_key_path"); });
                }
            }
            
            Checkbox {
                label: "验证密钥".to_string(),
                checked: config.verify_keys,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.federation.verify_keys = checked);
                }
            }
        }
    }
}

/// Authentication configuration form component
///
/// Manages user authentication settings including registration controls,
/// JWT secret and expiry configuration.
#[component]
fn AuthConfigForm(
    config: AuthConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "认证配置" }
            
            Checkbox {
                label: "启用用户注册".to_string(),
                checked: config.registration_enabled,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.auth.registration_enabled = checked);
                }
            }
            
            Input {
                label: "JWT 密钥".to_string(),
                input_type: "password".to_string(),
                value: config.jwt_secret.clone(),
                required: true,
                error: errors.get("auth.jwt_secret").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.auth.jwt_secret = val.clone());
                    validation_errors.with_mut(|errs| { errs.remove("auth.jwt_secret"); });
                }
            }
            
            Input {
                label: "JWT 过期时间 (秒)".to_string(),
                input_type: "number".to_string(),
                value: config.jwt_expiry.to_string(),
                required: true,
                error: errors.get("auth.jwt_expiry").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(expiry) = val.parse() {
                        form_data.with_mut(|cfg| cfg.auth.jwt_expiry = expiry);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("auth.jwt_expiry"); });
                }
            }
        }
    }
}

/// Media configuration form component
///
/// Controls media file handling including storage path, file size limits,
/// and URL preview settings.
#[component]
fn MediaConfigForm(
    config: MediaConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "媒体配置" }
            
            Input {
                label: "存储路径".to_string(),
                value: config.storage_path.clone(),
                required: true,
                error: errors.get("media.storage_path").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.media.storage_path = val.clone());
                    validation_errors.with_mut(|errs| { errs.remove("media.storage_path"); });
                }
            }
            
            Input {
                label: "最大文件大小 (字节)".to_string(),
                input_type: "number".to_string(),
                value: config.max_file_size.to_string(),
                required: true,
                error: errors.get("media.max_file_size").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(size) = val.parse() {
                        form_data.with_mut(|cfg| cfg.media.max_file_size = size);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("media.max_file_size"); });
                }
            }
            
            Checkbox {
                label: "启用 URL 预览".to_string(),
                checked: config.enable_url_previews,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.media.enable_url_previews = checked);
                }
            }
        }
    }
}

/// Network configuration form component
///
/// Manages network-related settings including timeouts, rate limiting,
/// and connection parameters.
#[component]
fn NetworkConfigForm(
    config: NetworkConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "网络配置" }
            
            Input {
                label: "请求超时 (秒)".to_string(),
                input_type: "number".to_string(),
                value: config.request_timeout.to_string(),
                required: true,
                error: errors.get("network.request_timeout").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(timeout) = val.parse() {
                        form_data.with_mut(|cfg| cfg.network.request_timeout = timeout);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("network.request_timeout"); });
                }
            }
            
            Input {
                label: "连接超时 (秒)".to_string(),
                input_type: "number".to_string(),
                value: config.connection_timeout.to_string(),
                required: true,
                error: errors.get("network.connection_timeout").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(timeout) = val.parse() {
                        form_data.with_mut(|cfg| cfg.network.connection_timeout = timeout);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("network.connection_timeout"); });
                }
            }
            
            h5 { class: "text-md font-medium text-gray-700 mt-4", "速率限制" }
            
            Checkbox {
                label: "启用速率限制".to_string(),
                checked: config.rate_limits.enabled,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.network.rate_limits.enabled = checked);
                }
            }
            
            Input {
                label: "每分钟请求数".to_string(),
                input_type: "number".to_string(),
                value: config.rate_limits.requests_per_minute.to_string(),
                required: true,
                error: errors.get("network.rate_limits.requests_per_minute").cloned(),
                oninput: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    if let Ok(rpm) = val.parse() {
                        form_data.with_mut(|cfg| cfg.network.rate_limits.requests_per_minute = rpm);
                    }
                    validation_errors.with_mut(|errs| { errs.remove("network.rate_limits.requests_per_minute"); });
                }
            }
        }
    }
}

/// Logging configuration form component
///
/// Controls logging behavior including log level, format selection,
/// and Prometheus metrics integration.
#[component]
fn LoggingConfigForm(
    config: LoggingConfigSection,
    errors: HashMap<String, String>,
    mut form_data: Signal<WebConfigData>,
    mut is_dirty: Signal<bool>,
    mut save_success: Signal<bool>,
    mut validation_errors: Signal<HashMap<String, String>>,
) -> Element {
    rsx! {
        div { class: "space-y-6",
            h4 { class: "text-lg font-medium text-gray-900", "日志配置" }
            
            Select {
                label: "日志级别".to_string(),
                value: format!("{:?}", config.level),
                options: vec![
                    ("Debug".to_string(), "Debug".to_string()),
                    ("Info".to_string(), "Info".to_string()),
                    ("Warn".to_string(), "Warn".to_string()),
                    ("Error".to_string(), "Error".to_string()),
                ],
                required: true,
                error: errors.get("logging.level").cloned(),
                onchange: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    let level = match val.as_str() {
                        "Debug" => LogLevel::Debug,
                        "Info" => LogLevel::Info,
                        "Warn" => LogLevel::Warn,
                        "Error" => LogLevel::Error,
                        _ => LogLevel::Info,
                    };
                    form_data.with_mut(|cfg| cfg.logging.level = level);
                    validation_errors.with_mut(|errs| { errs.remove("logging.level"); });
                }
            }
            
            Select {
                label: "日志格式".to_string(),
                value: format!("{:?}", config.format),
                options: vec![
                    ("Json".to_string(), "JSON".to_string()),
                    ("Pretty".to_string(), "Pretty".to_string()),
                    ("Compact".to_string(), "Compact".to_string()),
                    ("Text".to_string(), "Text".to_string()),
                ],
                required: true,
                error: errors.get("logging.format").cloned(),
                onchange: move |val: String| {
                    is_dirty.set(true);
                    save_success.set(false);
                    let format = match val.as_str() {
                        "Json" => LogFormat::Json,
                        "Pretty" => LogFormat::Pretty,
                        "Compact" => LogFormat::Compact,
                        "Text" => LogFormat::Text,
                        _ => LogFormat::Pretty,
                    };
                    form_data.with_mut(|cfg| cfg.logging.format = format);
                    validation_errors.with_mut(|errs| { errs.remove("logging.format"); });
                }
            }
            
            Checkbox {
                label: "启用 Prometheus 指标".to_string(),
                checked: config.prometheus_metrics,
                onchange: move |checked: bool| {
                    is_dirty.set(true);
                    save_success.set(false);
                    form_data.with_mut(|cfg| cfg.logging.prometheus_metrics = checked);
                }
            }
        }
    }
}
