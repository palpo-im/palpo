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

/// TOML Editor Component (Light Theme)
///
/// Provides a code editor for editing TOML configuration files with:
/// - Light-themed editor area
/// - Line numbers
/// - Undo/redo functionality
/// - Real-time validation
/// - Dirty state tracking
/// - Status bar with file info
pub fn TomlEditor(props: TomlEditorProps) -> Element {
    let mut undo_stack = use_signal(|| Vec::<String>::new());
    let mut redo_stack = use_signal(|| Vec::<String>::new());
    let mut current_content = use_signal(|| props.content.clone());

    let can_undo = !undo_stack.read().is_empty();
    let can_redo = !redo_stack.read().is_empty();

    let line_count = current_content.read().lines().count();
    let char_count = current_content.read().len();

    rsx! {
        div { class: "rounded-lg overflow-hidden border border-gray-300 shadow-sm flex flex-col h-full",
            // Toolbar
            div { class: "flex items-center justify-between px-3 py-2 border-b border-gray-200 bg-gray-50 flex-shrink-0",
                div { class: "flex items-center space-x-1",
                    // File icon and name
                    div { class: "flex items-center space-x-2 mr-3",
                        svg { class: "w-4 h-4 text-gray-500", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" }
                        }
                        span { class: "text-xs text-gray-600 font-medium", "palpo.toml" }
                    }
                    // Separator
                    div { class: "w-px h-4 bg-gray-300 mx-1" }
                    // Undo button
                    button {
                        class: "flex items-center space-x-1 px-2 py-1 text-xs rounded hover:bg-gray-200 transition-colors text-gray-600 hover:text-gray-900 disabled:opacity-40",
                        disabled: !can_undo,
                        onclick: move |_| {
                            if let Some(previous_content) = undo_stack.write().pop() {
                                redo_stack.with_mut(|stack| {
                                    stack.push(current_content.read().clone());
                                });
                                current_content.set(previous_content.clone());
                                props.onchange.call(previous_content);
                            }
                        },
                        title: "撤销",
                        svg { class: "w-3.5 h-3.5", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M3 10h10a5 5 0 015 5v2M3 10l4-4M3 10l4 4" }
                        }
                    }
                    // Redo button
                    button {
                        class: "flex items-center space-x-1 px-2 py-1 text-xs rounded hover:bg-gray-200 transition-colors text-gray-600 hover:text-gray-900 disabled:opacity-40",
                        disabled: !can_redo,
                        onclick: move |_| {
                            if let Some(next_content) = redo_stack.write().pop() {
                                undo_stack.with_mut(|stack| {
                                    stack.push(current_content.read().clone());
                                });
                                current_content.set(next_content.clone());
                                props.onchange.call(next_content);
                            }
                        },
                        title: "重做",
                        svg { class: "w-3.5 h-3.5", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M21 10H11a5 5 0 00-5 5v2M21 10l-4-4M21 10l-4 4" }
                        }
                    }
                }

                div { class: "flex items-center space-x-2",
                    // Reset button
                    button {
                        class: "flex items-center space-x-1 px-3 py-1 text-xs rounded border border-gray-300 hover:bg-gray-100 transition-colors text-gray-600 hover:text-gray-900 disabled:opacity-40",
                        disabled: !props.is_dirty,
                        onclick: move |_| {
                            current_content.set(props.content.clone());
                            undo_stack.set(Vec::new());
                            redo_stack.set(Vec::new());
                            props.onreset.call(());
                        },
                        svg { class: "w-3.5 h-3.5", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" }
                        }
                        span { "重置" }
                    }
                    // Save button
                    button {
                        class: "flex items-center space-x-1 px-3 py-1.5 text-xs rounded font-medium transition-colors",
                        class: if props.is_dirty && !props.is_loading {
                            "bg-blue-600 text-white hover:bg-blue-700 shadow-sm"
                        } else {
                            "bg-gray-300 text-gray-500 cursor-not-allowed"
                        },
                        disabled: !props.is_dirty || props.is_loading,
                        onclick: move |_| {
                            props.onsave.call(current_content.read().clone());
                        },
                        title: "保存",
                        if props.is_loading {
                            svg { class: "w-3.5 h-3.5 animate-spin", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24",
                                circle { class: "opacity-25", cx: "12", cy: "12", r: "10", stroke: "currentColor", stroke_width: "4" }
                                path { class: "opacity-75", fill: "currentColor", d: "M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" }
                            }
                            span { "保存中..." }
                        } else {
                            svg { class: "w-3.5 h-3.5", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                                path { stroke_linecap: "round", stroke_linejoin: "round", d: "M5 13l4 4L19 7" }
                            }
                            span { "保存" }
                        }
                    }
                }
            }

            // Editor container - light theme, flexible height
            div { class: "flex bg-white flex-1 min-h-0 overflow-hidden",
                // Line numbers gutter
                if props.show_line_numbers {
                    div { class: "select-none text-right pr-4 pl-4 py-3 text-xs leading-[1.6] font-mono text-gray-400 bg-gray-50 border-r border-gray-200 overflow-y-auto flex-shrink-0",
                        style: "min-width: 60px;",
                        for (i, _line) in current_content.read().lines().enumerate() {
                            div { key: "{i}",
                                class: "hover:text-gray-600 transition-colors",
                                "{i + 1}"
                            }
                        }
                    }
                }

                // Code textarea - flexible height
                textarea {
                    class: "flex-1 p-3 font-mono text-sm leading-[1.6] text-gray-800 bg-white border-none focus:outline-none resize-none",
                    style: "tab-size: 2; min-height: 100px;",
                    value: "{current_content()}",
                    oninput: move |evt| {
                        let new_content = evt.value().clone();
                        undo_stack.with_mut(|stack| {
                            stack.push(current_content.read().clone());
                            if stack.len() > 50 {
                                stack.remove(0);
                            }
                        });
                        redo_stack.set(Vec::new());
                        current_content.set(new_content.clone());
                        props.onchange.call(new_content);
                    },
                    spellcheck: "false",
                    autocomplete: "off",
                    autocorrect: "off",
                    autocapitalize: "off",
                    wrap: "off",
                }
            }

            // Status bar - light theme
            div { class: "flex items-center justify-between px-3 py-1.5 bg-gray-50 border-t border-gray-200 text-gray-500 text-xs select-none flex-shrink-0",
                div { class: "flex items-center space-x-4",
                    // Language indicator
                    div { class: "flex items-center space-x-1",
                        svg { class: "w-3 h-3", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" }
                        }
                        span { "TOML" }
                    }
                    span { "UTF-8" }
                }
                div { class: "flex items-center space-x-4",
                    span { "Ln {line_count}" }
                    span { "{char_count} 字符" }
                    // Dirty indicator
                    if props.is_dirty {
                        div { class: "flex items-center space-x-1 text-amber-600",
                            div { class: "w-1.5 h-1.5 rounded-full bg-amber-500" }
                            span { "已修改" }
                        }
                    } else {
                        div { class: "flex items-center space-x-1 text-green-600",
                            div { class: "w-1.5 h-1.5 rounded-full bg-green-500" }
                            span { "已保存" }
                        }
                    }
                }
            }

            // Validation errors panel
            if !props.errors.is_empty() {
                div { class: "bg-red-50 border-t border-red-200 p-3 flex-shrink-0",
                    div { class: "flex items-start space-x-2",
                        svg { class: "w-4 h-4 text-red-500 mt-0.5 flex-shrink-0", xmlns: "http://www.w3.org/2000/svg", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", stroke_width: "2",
                            path { stroke_linecap: "round", stroke_linejoin: "round", d: "M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" }
                        }
                        div { class: "space-y-1 flex-1",
                            p { class: "text-xs font-medium text-red-800", "验证错误" }
                            for (key, message) in props.errors.iter() {
                                p { key: "{key}", class: "text-xs text-red-600 font-mono",
                                    "{key}: {message}"
                                }
                            }
                        }
                    }
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
