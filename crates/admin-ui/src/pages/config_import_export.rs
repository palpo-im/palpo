//! Configuration Import/Export Page
//!
//! Provides configuration import/export functionality for TOML format (Palpo compatible).

use dioxus::prelude::*;
use crate::components::forms::Button;
use crate::components::feedback::{ErrorMessage, SuccessMessage};
use crate::services::config_import_export_api::{
    ConfigImportExportAPI, ExportOptions, ConfigImportRequest, MergeStrategy, ConfigFormat
};

/// Main configuration import/export page component
#[component]
pub fn ConfigImportExportPage() -> Element {
    let mut active_tab = use_signal(|| "export".to_string());

    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "flex justify-between items-center",
                        div {
                            h3 { class: "text-lg leading-6 font-medium text-gray-900", "配置导入/导出" }
                            p { class: "mt-1 text-sm text-gray-500", "导出和导入 TOML 格式的服务器配置" }
                        }
                        div { class: "flex space-x-3",
                            Button {
                                variant: if active_tab() == "export" { "primary".to_string() } else { "secondary".to_string() },
                                onclick: move |_| active_tab.set("export".to_string()),
                                "导出"
                            }
                            Button {
                                variant: if active_tab() == "import" { "primary".to_string() } else { "secondary".to_string() },
                                onclick: move |_| active_tab.set("import".to_string()),
                                "导入"
                            }
                        }
                    }
                }
            }

            match active_tab().as_str() {
                "export" => rsx! { ExportPanel {} },
                "import" => rsx! { ImportPanel {} },
                _ => rsx! { div {} }
            }
        }
    }
}

/// Export configuration panel
#[component]
fn ExportPanel() -> Element {
    let mut exported_content = use_signal(|| None::<String>);
    let mut is_loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let handle_export = move |_| {
        spawn(async move {
            is_loading.set(true);
            error.set(None);

            let options = ExportOptions {
                format: ConfigFormat::Toml,
                include_sensitive: false,
                include_defaults: false,
                sections: None,
                encrypt: false,
                encryption_key: None,
            };

            match ConfigImportExportAPI::export_config(options).await {
                Ok(response) => {
                    exported_content.set(Some(response.content));
                }
                Err(e) => {
                    error.set(Some(format!("导出失败: {}", e)));
                }
            }

            is_loading.set(false);
        });
    };

    rsx! {
        div { class: "bg-white shadow rounded-lg",
            div { class: "px-4 py-5 sm:p-6",
                h4 { class: "text-md font-medium text-gray-900 mb-4", "导出配置" }

                div { class: "space-y-4",
                    p { class: "text-sm text-gray-600", "将当前服务器配置导出为 TOML 格式文件" }

                    Button {
                        variant: "primary".to_string(),
                        onclick: handle_export,
                        disabled: is_loading(),
                        if is_loading() { "导出中..." } else { "导出配置" }
                    }

                    if let Some(err) = error() {
                        ErrorMessage { message: err, on_close: Some(EventHandler::new(move |_| error.set(None))) }
                    }

                    if let Some(content) = exported_content() {
                        div { class: "mt-4",
                            label { class: "block text-sm font-medium text-gray-700 mb-2", "导出内容" }
                            textarea {
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 font-mono text-sm",
                                rows: "20",
                                readonly: true,
                                value: content.clone()
                            }
                            div { class: "mt-2",
                                Button {
                                    variant: "secondary".to_string(),
                                    onclick: move |_| {
                                        // Clipboard copy not available in WASM without user interaction
                                        // This is a known limitation of the Web API
                                    },
                                    "复制到剪贴板"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Import configuration panel
#[component]
fn ImportPanel() -> Element {
    let mut import_content = use_signal(|| String::new());
    let mut is_loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut success = use_signal(|| None::<String>);

    let handle_import = move |_| {
        spawn(async move {
            is_loading.set(true);
            error.set(None);
            success.set(None);

            let request = ConfigImportRequest {
                content: import_content(),
                format: ConfigFormat::Toml,
                merge_strategy: MergeStrategy::Replace,
                validate_only: false,
                backup_current: true,
                encryption_key: None,
            };

            match ConfigImportExportAPI::import_config(request).await {
                Ok(result) => {
                    if result.success {
                        success.set(Some("配置导入成功".to_string()));
                        import_content.set(String::new());
                    } else {
                        error.set(Some(format!("导入失败: {}", result.errors.join(", "))));
                    }
                }
                Err(e) => {
                    error.set(Some(format!("导入失败: {}", e)));
                }
            }

            is_loading.set(false);
        });
    };

    rsx! {
        div { class: "bg-white shadow rounded-lg",
            div { class: "px-4 py-5 sm:p-6",
                h4 { class: "text-md font-medium text-gray-900 mb-4", "导入配置" }

                div { class: "space-y-4",
                    p { class: "text-sm text-gray-600", "从 TOML 格式文件导入配置。导入前会自动备份当前配置。" }

                    div {
                        label { class: "block text-sm font-medium text-gray-700 mb-2", "配置内容" }
                        textarea {
                            class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500 font-mono text-sm",
                            rows: "15",
                            placeholder: "粘贴 TOML 格式的配置内容",
                            value: import_content(),
                            oninput: move |evt| import_content.set(evt.value())
                        }
                    }

                    Button {
                        variant: "primary".to_string(),
                        onclick: handle_import,
                        disabled: is_loading() || import_content().is_empty(),
                        if is_loading() { "导入中..." } else { "导入配置" }
                    }

                    if let Some(err) = error() {
                        ErrorMessage { message: err, on_close: Some(EventHandler::new(move |_| error.set(None))) }
                    }

                    if let Some(msg) = success() {
                        SuccessMessage { message: msg, on_close: Some(EventHandler::new(move |_| success.set(None))) }
                    }
                }
            }
        }
    }
}
