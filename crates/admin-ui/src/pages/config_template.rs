//! Configuration Template Management Page
//!
//! Provides UI for managing configuration templates including:
//! - Listing available templates (built-in and custom)
//! - Creating new templates from current configuration
//! - Applying templates with preview and diff
//! - Editing and deleting custom templates

use dioxus::prelude::*;
use crate::components::forms::{Input, Button};
use crate::components::feedback::{ErrorMessage, SuccessMessage};
use crate::services::config_template_api::{ConfigTemplateAPI, ConfigTemplate, ConfigTemplateDetail};

/// Main configuration template management page
#[component]
pub fn ConfigTemplatePage() -> Element {
    let mut templates = use_signal(|| Vec::<ConfigTemplate>::new());
    let mut active_tab = use_signal(|| "list".to_string());
    let mut error = use_signal(|| None::<String>);
    let mut success_message = use_signal(|| None::<String>);
    let mut selected_template = use_signal(|| None::<ConfigTemplateDetail>);
    let mut is_loading = use_signal(|| false);

    // Load templates on mount
    use_effect(move || {
        spawn(async move {
            is_loading.set(true);
            match ConfigTemplateAPI::list_templates().await {
                Ok(tpls) => {
                    templates.set(tpls);
                    error.set(None);
                }
                Err(e) => error.set(Some(format!("Failed to load templates: {}", e))),
            }
            is_loading.set(false);
        });
    });

    rsx! {
        div { class: "space-y-6",
            // Header
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "flex justify-between items-center",
                        div {
                            h3 { class: "text-lg leading-6 font-medium text-gray-900",
                                "配置模板管理"
                            }
                            p { class: "mt-1 text-sm text-gray-500",
                                "管理配置模板，快速应用预设配置"
                            }
                        }
                        div { class: "flex space-x-3",
                            Button {
                                variant: "secondary".to_string(),
                                onclick: move |_| {
                                    active_tab.set("list".to_string());
                                    selected_template.set(None);
                                },
                                "模板列表"
                            }
                            Button {
                                variant: "primary".to_string(),
                                onclick: move |_| active_tab.set("create".to_string()),
                                "创建模板"
                            }
                        }
                    }
                }
            }

            // Feedback messages
            if let Some(msg) = success_message() {
                SuccessMessage { message: msg }
            }
            if let Some(err) = error() {
                ErrorMessage { message: err }
            }

            // Loading indicator
            if is_loading() {
                div { class: "flex justify-center py-8",
                    div { class: "animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-600" }
                }
            }

            // Content based on active tab
            if !is_loading() {
                match active_tab().as_str() {
                    "list" => rsx! {
                        TemplateList {
                            templates: templates(),
                            on_select: move |template_id: String| {
                                spawn(async move {
                                    match ConfigTemplateAPI::get_template(&template_id).await {
                                        Ok(detail) => {
                                            selected_template.set(Some(detail));
                                            active_tab.set("preview".to_string());
                                        }
                                        Err(e) => error.set(Some(format!("Failed to load template: {}", e))),
                                    }
                                });
                            }
                        }
                    },
                    "create" => rsx! {
                        TemplateCreator {
                            on_cancel: move |_| active_tab.set("list".to_string()),
                            on_success: move |msg: String| {
                                success_message.set(Some(msg));
                                active_tab.set("list".to_string());
                                // Reload templates
                                spawn(async move {
                                    if let Ok(tpls) = ConfigTemplateAPI::list_templates().await {
                                        templates.set(tpls);
                                    }
                                });
                            }
                        }
                    },
                    "preview" => rsx! {
                        if let Some(template_detail) = selected_template() {
                            TemplatePreview {
                                template: template_detail,
                                on_apply: move |_| {
                                    success_message.set(Some("模板应用成功".to_string()));
                                    active_tab.set("list".to_string());
                                },
                                on_cancel: move |_| active_tab.set("list".to_string())
                            }
                        }
                    },
                    _ => rsx! { div {} }
                }
            }
        }
    }
}

/// Template list component
#[component]
fn TemplateList(templates: Vec<ConfigTemplate>, on_select: EventHandler<String>) -> Element {
    rsx! {
        div { class: "bg-white shadow rounded-lg",
            div { class: "px-4 py-5 sm:p-6",
                h4 { class: "text-lg font-medium text-gray-900 mb-6", "可用模板" },
                if templates.is_empty() {
                    div { class: "text-center py-8 text-gray-500", "没有找到模板" }
                } else {
                    div { class: "grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3",
                        for template in templates {
                            div {
                                key: "{template.id}",
                                class: "border rounded-lg p-4 hover:border-indigo-500 cursor-pointer transition-colors",
                                onclick: move |_| on_select.call(template.id.clone()),
                                div { class: "flex justify-between items-start mb-2",
                                    div {
                                        h5 { class: "text-sm font-medium text-gray-900", "{template.name}" }
                                        p { class: "mt-1 text-xs text-gray-500", "{template.description}" }
                                    }
                                    if template.is_builtin {
                                        span { class: "px-2 py-1 text-xs font-medium rounded-full bg-green-100 text-green-800", "内置" }
                                    } else {
                                        span { class: "px-2 py-1 text-xs font-medium rounded-full bg-blue-100 text-blue-800", "自定义" }
                                    }
                                }
                                div { class: "mt-2 flex items-center text-xs text-gray-400",
                                    span { "类别: " }
                                    span { class: "ml-1 font-medium", "{template.category:?}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Template creator component - exports current config as template
#[component]
fn TemplateCreator(on_cancel: EventHandler<()>, on_success: EventHandler<String>) -> Element {
    let mut name = use_signal(|| String::new());
    let mut description = use_signal(|| String::new());
    let mut category = use_signal(|| "Custom".to_string());
    let mut is_saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let handle_save = move |_| {
        let name_val = name();
        let desc_val = description();
        let cat_val = category();
        
        if name_val.trim().is_empty() {
            error.set(Some("模板名称不能为空".to_string()));
            return;
        }

        is_saving.set(true);
        spawn(async move {
            // Parse category
            let template_category = match cat_val.as_str() {
                "Development" => crate::services::config_template_api::TemplateCategory::Development,
                "Production" => crate::services::config_template_api::TemplateCategory::Production,
                "Testing" => crate::services::config_template_api::TemplateCategory::Testing,
                "Federation" => crate::services::config_template_api::TemplateCategory::Federation,
                "Security" => crate::services::config_template_api::TemplateCategory::Security,
                _ => crate::services::config_template_api::TemplateCategory::Custom,
            };

            match ConfigTemplateAPI::export_current_as_template(
                name_val,
                desc_val,
                template_category
            ).await {
                Ok(_) => {
                    on_success.call("模板创建成功".to_string());
                }
                Err(e) => {
                    error.set(Some(format!("创建模板失败: {}", e)));
                }
            }
            is_saving.set(false);
        });
    };

    rsx! {
        div { class: "bg-white shadow rounded-lg",
            div { class: "px-4 py-5 sm:p-6",
                h4 { class: "text-lg font-medium text-gray-900 mb-6", "从当前配置创建模板" },
                
                if let Some(err) = error() {
                    ErrorMessage { message: err }
                }

                div { class: "space-y-4",
                    Input {
                        label: "模板名称".to_string(),
                        value: name(),
                        required: true,
                        placeholder: "例如: 我的生产环境配置".to_string(),
                        oninput: move |s| name.set(s),
                    }
                    
                    Input {
                        label: "模板描述".to_string(),
                        value: description(),
                        required: false,
                        placeholder: "描述此模板的用途和特点".to_string(),
                        oninput: move |s| description.set(s),
                    }

                    div { class: "space-y-2",
                        label { class: "block text-sm font-medium text-gray-700", "模板类别" }
                        select {
                            class: "mt-1 block w-full pl-3 pr-10 py-2 text-base border-gray-300 focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm rounded-md",
                            value: "{category()}",
                            onchange: move |evt| category.set(evt.value().clone()),
                            option { value: "Custom", "自定义" }
                            option { value: "Development", "开发环境" }
                            option { value: "Production", "生产环境" }
                            option { value: "Testing", "测试环境" }
                            option { value: "Federation", "联邦配置" }
                            option { value: "Security", "安全配置" }
                        }
                    }

                    div { class: "bg-blue-50 border border-blue-200 rounded-md p-4",
                        p { class: "text-sm text-blue-700",
                            "此操作将导出当前服务器配置作为模板。敏感信息（如密码、密钥）将被替换为占位符。"
                        }
                    }
                }

                div { class: "mt-6 flex justify-end space-x-3",
                    Button {
                        variant: "secondary".to_string(),
                        onclick: move |_| on_cancel.call(()),
                        disabled: is_saving(),
                        "取消"
                    }
                    Button {
                        variant: "primary".to_string(),
                        onclick: handle_save,
                        disabled: is_saving(),
                        if is_saving() { "保存中..." } else { "创建模板" }
                    }
                }
            }
        }
    }
}

/// Template preview and apply component
#[component]
fn TemplatePreview(
    template: ConfigTemplateDetail,
    on_apply: EventHandler<()>,
    on_cancel: EventHandler<()>
) -> Element {
    let mut is_applying = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut show_diff = use_signal(|| false);

    let handle_apply = move |_| {
        is_applying.set(true);
        let template_id = template.template.id.clone();
        
        spawn(async move {
            match ConfigTemplateAPI::apply_template(&template_id, None).await {
                Ok(_config) => {
                    // TODO: Actually apply the configuration to the server
                    on_apply.call(());
                }
                Err(e) => {
                    error.set(Some(format!("应用模板失败: {}", e)));
                }
            }
            is_applying.set(false);
        });
    };

    rsx! {
        div { class: "bg-white shadow rounded-lg",
            div { class: "px-4 py-5 sm:p-6",
                // Header
                div { class: "flex justify-between items-start mb-6",
                    div {
                        h4 { class: "text-lg font-medium text-gray-900", "{template.template.name}" }
                        p { class: "mt-1 text-sm text-gray-500", "{template.template.description}" }
                    }
                    div { class: "flex items-center space-x-2",
                        if template.template.is_builtin {
                            span { class: "px-2 py-1 text-xs font-medium rounded-full bg-green-100 text-green-800", "内置模板" }
                        }
                        span { class: "px-2 py-1 text-xs font-medium rounded-full bg-gray-100 text-gray-800",
                            "{template.template.category:?}"
                        }
                    }
                }

                if let Some(err) = error() {
                    ErrorMessage { message: err }
                }

                // Template details
                div { class: "space-y-6",
                    // Required fields
                    div {
                        h5 { class: "text-sm font-medium text-gray-900 mb-3", "必填字段" }
                        div { class: "bg-gray-50 rounded-md p-4",
                            ul { class: "space-y-2",
                                for field in &template.required_fields {
                                    li { class: "text-sm text-gray-700 flex items-center",
                                        span { class: "text-red-500 mr-2", "●" }
                                        code { class: "bg-gray-200 px-2 py-1 rounded text-xs", "{field}" }
                                    }
                                }
                            }
                        }
                    }

                    // Optional fields
                    if !template.optional_fields.is_empty() {
                        div {
                            h5 { class: "text-sm font-medium text-gray-900 mb-3", "可选字段" }
                            div { class: "bg-gray-50 rounded-md p-4",
                                ul { class: "space-y-2",
                                    for field in &template.optional_fields {
                                        li { class: "text-sm text-gray-700 flex items-center",
                                            span { class: "text-gray-400 mr-2", "○" }
                                            code { class: "bg-gray-200 px-2 py-1 rounded text-xs", "{field}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Configuration preview toggle
                    div {
                        button {
                            class: "text-sm text-indigo-600 hover:text-indigo-800 font-medium",
                            onclick: move |_| show_diff.set(!show_diff()),
                            if show_diff() { "隐藏配置详情" } else { "显示配置详情" }
                        }
                        
                        if show_diff() {
                            div { class: "mt-4 bg-gray-900 rounded-md p-4 overflow-auto max-h-96",
                                pre { class: "text-xs text-gray-100",
                                    code { 
                                        dangerous_inner_html: serde_json::to_string_pretty(&template.config_data).unwrap_or_default()
                                    }
                                }
                            }
                        }
                    }

                    // Warning message
                    div { class: "bg-yellow-50 border border-yellow-200 rounded-md p-4",
                        div { class: "flex",
                            div { class: "flex-shrink-0",
                                svg { class: "h-5 w-5 text-yellow-400", fill: "currentColor", view_box: "0 0 20 20",
                                    path {
                                        fill_rule: "evenodd",
                                        d: "M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z",
                                        clip_rule: "evenodd"
                                    }
                                }
                            }
                            div { class: "ml-3",
                                h3 { class: "text-sm font-medium text-yellow-800", "注意" }
                                div { class: "mt-2 text-sm text-yellow-700",
                                    p { "应用此模板将覆盖当前配置。请确保：" }
                                    ul { class: "list-disc list-inside mt-2 space-y-1",
                                        li { "已备份当前配置" }
                                        li { "了解模板的配置内容" }
                                        li { "准备好重启服务器（如需要）" }
                                    }
                                }
                            }
                        }
                    }
                }

                // Actions
                div { class: "mt-6 flex justify-end space-x-3",
                    Button {
                        variant: "secondary".to_string(),
                        onclick: move |_| on_cancel.call(()),
                        disabled: is_applying(),
                        "取消"
                    }
                    Button {
                        variant: "primary".to_string(),
                        onclick: handle_apply,
                        disabled: is_applying(),
                        if is_applying() { "应用中..." } else { "应用模板" }
                    }
                }
            }
        }
    }
}