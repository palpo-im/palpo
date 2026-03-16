use dioxus::prelude::*;
use std::collections::HashMap;
use crate::components::toml_editor::TomlEditor;
use crate::components::feedback::{ErrorMessage, SuccessMessage};
use crate::components::loading::Spinner;
use crate::services::config_api::ConfigAPI;

/// TOML Editor Page Component
/// 
/// Allows users to directly edit the TOML configuration file with:
/// - Raw TOML content display
/// - Syntax highlighting and line numbers
/// - Undo/redo functionality
/// - Real-time TOML validation
/// - Save/reset functionality
/// - Ctrl+S keyboard shortcut support
pub fn ConfigTomlEditorPage() -> Element {
    let mut toml_content = use_signal(|| String::new());
    let mut is_loading = use_signal(|| true);
    let mut is_saving = use_signal(|| false);
    let mut is_dirty = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);
    let mut validation_errors = use_signal(|| HashMap::<String, String>::new());

    // Load TOML content on mount
    {
        use_effect(move || {
            let load_toml = async move {
                is_loading.set(true);
                match ConfigAPI::get_toml_content().await {
                    Ok(content) => {
                        toml_content.set(content);
                        is_dirty.set(false);
                        error.set(None);
                    }
                    Err(e) => {
                        error.set(Some(format!("加载 TOML 文件失败: {}", e)));
                    }
                }
                is_loading.set(false);
            };
            
            spawn(load_toml);
        });
    }

    // Handle content change
    let handle_content_change = move |new_content: String| {
        toml_content.set(new_content);
        is_dirty.set(true);
        success_message.set(None);
    };

    // Handle validation
    let handle_validate = {
        move |_| {
            let content = toml_content.read().clone();
            let validate_toml = async move {
                match ConfigAPI::validate_toml(&content).await {
                    Ok(result) => {
                        if result.is_valid {
                            validation_errors.set(HashMap::new());
                            success_message.set(Some("TOML 格式有效".to_string()));
                            error.set(None);
                        } else {
                            let mut errors = HashMap::new();
                            if let Some(msg) = result.error_message {
                                errors.insert("syntax".to_string(), msg);
                            }
                            validation_errors.set(errors);
                            error.set(Some("TOML 验证失败".to_string()));
                        }
                    }
                    Err(e) => {
                        error.set(Some(format!("验证失败: {}", e)));
                    }
                }
            };
            
            spawn(validate_toml);
        }
    };

    // Handle save
    let handle_save = {
        move |_| {
            let content = toml_content.read().clone();
            let save_toml = async move {
                is_saving.set(true);
                
                // First validate
                match ConfigAPI::validate_toml(&content).await {
                    Ok(result) => {
                        if !result.is_valid {
                            error.set(Some("TOML 验证失败，无法保存".to_string()));
                            is_saving.set(false);
                            return;
                        }
                    }
                    Err(e) => {
                        error.set(Some(format!("验证失败: {}", e)));
                        is_saving.set(false);
                        return;
                    }
                }
                
                // Then save
                match ConfigAPI::save_toml_content(&content).await {
                    Ok(_) => {
                        is_dirty.set(false);
                        success_message.set(Some("TOML 文件已保存".to_string()));
                        error.set(None);
                        validation_errors.set(HashMap::new());
                    }
                    Err(e) => {
                        error.set(Some(format!("保存失败: {}", e)));
                    }
                }
                
                is_saving.set(false);
            };
            
            spawn(save_toml);
        }
    };

    // Handle reset
    let handle_reset = {
        move |_| {
            let reset_toml = async move {
                is_loading.set(true);
                match ConfigAPI::get_toml_content().await {
                    Ok(content) => {
                        toml_content.set(content);
                        is_dirty.set(false);
                        error.set(None);
                        success_message.set(None);
                        validation_errors.set(HashMap::new());
                    }
                    Err(e) => {
                        error.set(Some(format!("重置失败: {}", e)));
                    }
                }
                is_loading.set(false);
            };
            
            spawn(reset_toml);
        }
    };

    rsx! {
        div { class: "space-y-6",
            // Header
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "flex justify-between items-center",
                        div {
                            h3 { class: "text-lg leading-6 font-medium text-gray-900",
                                "TOML 编辑器"
                            }
                            p { class: "mt-1 text-sm text-gray-500",
                                "直接编辑 palpo.toml 配置文件"
                            }
                        }
                        div { class: "flex space-x-3",
                            button {
                                class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50",
                                onclick: handle_validate,
                                "验证 TOML"
                            }
                        }
                    }
                    
                    // Messages
                    if let Some(msg) = success_message() {
                        div { class: "mt-4",
                            SuccessMessage { message: msg }
                        }
                    }
                    if let Some(err) = error() {
                        div { class: "mt-4",
                            ErrorMessage { message: err }
                        }
                    }
                }
            }

            // Editor
            if is_loading() {
                div { class: "bg-white shadow rounded-lg p-12",
                    div { class: "flex justify-center",
                        Spinner { size: "large".to_string() }
                    }
                }
            } else {
                div { class: "bg-white shadow rounded-lg",
                    TomlEditor {
                        content: toml_content(),
                        onchange: handle_content_change,
                        onsave: handle_save,
                        onreset: handle_reset,
                        errors: validation_errors(),
                        is_dirty: is_dirty(),
                        is_loading: is_saving(),
                        show_line_numbers: true,
                    }
                }
            }
        }
    }
}
