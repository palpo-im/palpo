//! Configuration Import/Export Page
//!
//! Provides comprehensive configuration import/export functionality with support for
//! multiple formats, options configuration, preview, and conflict resolution.

use dioxus::prelude::*;
use crate::components::forms::{Button, TextArea, Select};
use crate::components::feedback::{ErrorMessage, SuccessMessage};
use crate::services::config_import_export_api::{
    ConfigImportExportAPI, ConfigFormat, MergeStrategy, ExportOptions, ConfigImportRequest
};

/// Main configuration import/export page component
#[component]
pub fn ConfigImportExportPage() -> Element {
    let mut active_tab = use_signal(|| "export".to_string());
    let mut error = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);

    rsx! {
        div { class: "space-y-6",
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "flex justify-between items-center",
                        div {
                            h3 { class: "text-lg leading-6 font-medium text-gray-900", "配置导入/导出" }
                            p { class: "mt-1 text-sm text-gray-500", "导出、导入和管理服务器配置" }
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

            if let Some(msg) = success_message() {
                SuccessMessage { message: msg, on_close: Some(EventHandler::new(move |_| success_message.set(None))) }
            }
            if let Some(err) = error() {
                ErrorMessage { message: err, on_close: Some(EventHandler::new(move |_| error.set(None))) }
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
    let mut format = use_signal(|| ConfigFormat::Toml);
    let mut include_sensitive = use_signal(|| false);
    let mut exported_content = use_signal(|| None::<String>);
    let mut is_loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let handle_export = move |_| {
        spawn(async move {
            is_loading.set(true);
            error.set(None);

            let options = ExportOptions {
                format: format(),
                include_sensitive: include_sensitive(),
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
                    Select {
                        label: "导出格式".to_string(),
                        value: format!("{:?}", format()),
                        options: vec![
                            ("Toml".to_string(), "TOML".to_string(), None),
                            ("Json".to_string(), "JSON".to_string(), None),
                            ("Yaml".to_string(), "YAML".to_string(), None),
                        ],
                        onchange: move |val: String| {
                            format.set(match val.as_str() {
                                "Json" => ConfigFormat::Json,
                                "Yaml" => ConfigFormat::Yaml,
                                _ => ConfigFormat::Toml,
                            });
                        }
                    }

                    div { class: "flex items-center",
                        input {
                            r#type: "checkbox",
                            id: "include_sensitive",
                            checked: include_sensitive(),
                            onchange: move |evt| include_sensitive.set(evt.checked())
                        }
                        label {
                            r#for: "include_sensitive",
                            class: "ml-2 text-sm text-gray-700",
                            "包含敏感数据（密码、密钥等）"
                        }
                    }

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
                            TextArea {
                                label: "导出内容".to_string(),
                                value: content.clone(),
                                rows: 20.0,
                                readonly: true,
                                oninput: move |_| {}
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
    let mut merge_strategy = use_signal(|| MergeStrategy::Replace);
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
                merge_strategy: merge_strategy(),
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
                        error.set(Some(format!("导入失败: {:?}", result.errors)));
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
                    TextArea {
                        label: "配置内容".to_string(),
                        value: import_content(),
                        rows: 15.0,
                        placeholder: "粘贴配置内容（TOML、JSON 或 YAML 格式）".to_string(),
                        oninput: move |val: String| import_content.set(val)
                    }

                    Select {
                        label: "合并策略".to_string(),
                        value: format!("{:?}", merge_strategy()),
                        options: vec![
                            ("Replace".to_string(), "替换（覆盖现有配置）".to_string(), None),
                            ("Merge".to_string(), "合并（保留未修改的配置）".to_string(), None),
                        ],
                        onchange: move |val: String| {
                            merge_strategy.set(match val.as_str() {
                                "Merge" => MergeStrategy::Merge,
                                _ => MergeStrategy::Replace,
                            });
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
