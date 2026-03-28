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
        move |_: Event<MouseEvent>| {
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
        move |content: String| {
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
        move |_: ()| {
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
        div { class: "flex flex-col h-full min-h-0",
            // Editor - Takes full height
            if is_loading() {
                div { class: "flex-1 flex items-center justify-center",
                    Spinner { size: "large".to_string() }
                }
            } else {
                div { class: "flex-1 min-h-0",
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
            
            // Messages at bottom
            if let Some(msg) = success_message() {
                div { class: "flex-shrink-0 mt-2",
                    SuccessMessage { message: msg }
                }
            }
            if let Some(err) = error() {
                div { class: "flex-shrink-0 mt-2",
                    ErrorMessage { message: err }
                }
            }
        }
    }
}
