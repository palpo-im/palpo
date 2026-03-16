use dioxus::prelude::*;
use std::collections::HashMap;

/// Props for the TOML editor component
#[derive(Clone, Props, PartialEq)]
pub struct TomlEditorProps {
    /// Current TOML content
    pub content: String,
    /// Callback when content changes
    pub onchange: EventHandler<String>,
    /// Callback when save is requested
    pub onsave: EventHandler<String>,
    /// Callback when reset is requested
    pub onreset: EventHandler<()>,
    /// Validation errors with line/column info
    #[props(default = HashMap::new())]
    pub errors: HashMap<String, String>,
    /// Whether the editor is in a dirty state
    #[props(default = false)]
    pub is_dirty: bool,
    /// Whether the editor is loading
    #[props(default = false)]
    pub is_loading: bool,
    /// Whether to show line numbers
    #[props(default = true)]
    pub show_line_numbers: bool,
}

/// TOML Editor Component
/// 
/// Provides a code editor for editing TOML configuration files with:
/// - Syntax highlighting
/// - Line numbers
/// - Undo/redo functionality
/// - Real-time validation
/// - Dirty state tracking
pub fn TomlEditor(props: TomlEditorProps) -> Element {
    let mut undo_stack = use_signal(|| Vec::<String>::new());
    let mut redo_stack = use_signal(|| Vec::<String>::new());
    let mut current_content = use_signal(|| props.content.clone());

    let can_undo = !undo_stack.read().is_empty();
    let can_redo = !redo_stack.read().is_empty();

    rsx! {
        div { class: "space-y-4",
            // Toolbar
            div { class: "flex items-center justify-between bg-gray-50 border border-gray-200 rounded-t-lg p-3",
                div { class: "flex space-x-2",
                    button {
                        class: if can_undo { "btn btn-sm btn-secondary" } else { "btn btn-sm btn-secondary disabled" },
                        disabled: !can_undo,
                        onclick: move |_| {
                            if let Some(previous_content) = undo_stack.write().pop() {
                                // Push current content to redo stack
                                redo_stack.with_mut(|stack| {
                                    stack.push(current_content.read().clone());
                                });
                                
                                current_content.set(previous_content.clone());
                                props.onchange.call(previous_content);
                            }
                        },
                        title: "撤销 (Ctrl+Z)",
                        "↶ 撤销"
                    }
                    button {
                        class: if can_redo { "btn btn-sm btn-secondary" } else { "btn btn-sm btn-secondary disabled" },
                        disabled: !can_redo,
                        onclick: move |_| {
                            if let Some(next_content) = redo_stack.write().pop() {
                                // Push current content to undo stack
                                undo_stack.with_mut(|stack| {
                                    stack.push(current_content.read().clone());
                                });
                                
                                current_content.set(next_content.clone());
                                props.onchange.call(next_content);
                            }
                        },
                        title: "重做 (Ctrl+Y)",
                        "↷ 重做"
                    }
                }
                div { class: "flex space-x-2",
                    button {
                        class: if props.is_dirty { "btn btn-sm btn-secondary" } else { "btn btn-sm btn-secondary disabled" },
                        disabled: !props.is_dirty,
                        onclick: move |_| {
                            current_content.set(props.content.clone());
                            undo_stack.set(Vec::new());
                            redo_stack.set(Vec::new());
                            props.onreset.call(());
                        },
                        "重置"
                    }
                    button {
                        class: if props.is_dirty && !props.is_loading { "btn btn-sm btn-primary" } else { "btn btn-sm btn-primary disabled" },
                        disabled: !props.is_dirty || props.is_loading,
                        onclick: move |_| {
                            props.onsave.call(current_content.read().clone());
                        },
                        title: "保存 (Ctrl+S)",
                        if props.is_loading { "保存中..." } else { "保存" }
                    }
                }
            }

            // Editor container
            div { class: "border border-gray-200 rounded-b-lg overflow-hidden bg-white",
                div { class: "flex",
                    // Line numbers
                    if props.show_line_numbers {
                        div { class: "bg-gray-50 border-r border-gray-200 px-3 py-2 text-right text-xs text-gray-500 font-mono select-none overflow-hidden",
                            style: "width: 50px; line-height: 1.5;",
                            for (i, _) in current_content.read().lines().enumerate() {
                                div { key: "{i}", "{i + 1}" }
                            }
                        }
                    }

                    // Code editor
                    textarea {
                        class: "flex-1 p-3 font-mono text-sm border-none focus:outline-none resize-none",
                        style: "line-height: 1.5; tab-size: 2;",
                        value: "{current_content()}",
                        oninput: move |evt| {
                            let new_content = evt.value().clone();
                            // Push current content to undo stack
                            undo_stack.with_mut(|stack| {
                                stack.push(current_content.read().clone());
                                // Limit undo stack to 50 items
                                if stack.len() > 50 {
                                    stack.remove(0);
                                }
                            });
                            
                            // Clear redo stack on new change
                            redo_stack.set(Vec::new());
                            
                            current_content.set(new_content.clone());
                            props.onchange.call(new_content);
                        },
                        rows: "20",
                        spellcheck: "false",
                    }
                }
            }

            // Validation errors
            if !props.errors.is_empty() {
                div { class: "bg-red-50 border border-red-200 rounded-lg p-4",
                    h4 { class: "text-sm font-medium text-red-900 mb-2", "验证错误" }
                    div { class: "space-y-1",
                        for (key, message) in props.errors.iter() {
                            div { key: "{key}", class: "text-sm text-red-700",
                                "{key}: {message}"
                            }
                        }
                    }
                }
            }

            // Dirty state indicator
            if props.is_dirty {
                div { class: "text-xs text-amber-600 flex items-center",
                    span { class: "inline-block w-2 h-2 bg-amber-600 rounded-full mr-2" }
                    "存在未保存的修改"
                }
            }
        }
    }
}

/// Component to display TOML validation errors with line/column information
#[derive(Clone, Props, PartialEq)]
pub struct TomlValidationErrorProps {
    /// Error message
    pub message: String,
    /// Line number (1-indexed)
    #[props(default = None)]
    pub line: Option<usize>,
    /// Column number (1-indexed)
    #[props(default = None)]
    pub column: Option<usize>,
}

/// TOML Validation Error Display Component
pub fn TomlValidationError(props: TomlValidationErrorProps) -> Element {
    rsx! {
        div { class: "bg-red-50 border border-red-200 rounded p-3 text-sm",
            div { class: "text-red-900 font-medium", "{props.message}" }
            if let (Some(line), Some(column)) = (props.line, props.column) {
                div { class: "text-red-700 text-xs mt-1",
                    "位置: 第 {line} 行, 第 {column} 列"
                }
            }
        }
    }
}
