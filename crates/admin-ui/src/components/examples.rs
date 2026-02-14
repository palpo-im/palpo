//! Example usage of UI components
//! This module demonstrates how to use the common UI components

use dioxus::prelude::*;
use super::forms::*;
use super::feedback::*;
use super::loading::*;

/// Example form demonstrating all form components
#[component]
pub fn ExampleForm() -> Element {
    let mut username = use_signal(|| String::new());
    let mut password = use_signal(|| String::new());
    let mut bio = use_signal(|| String::new());
    let mut role = use_signal(|| "user".to_string());
    let mut agree_terms = use_signal(|| false);
    let mut loading = use_signal(|| false);
    let mut errors = use_signal(|| Vec::<String>::new());

    let handle_submit = move |_| {
        loading.set(true);
        errors.set(Vec::new());

        // Validation
        let mut validation_errors = Vec::new();
        if username().is_empty() {
            validation_errors.push("用户名不能为空".to_string());
        }
        if password().len() < 6 {
            validation_errors.push("密码至少需要6个字符".to_string());
        }
        if !agree_terms() {
            validation_errors.push("必须同意服务条款".to_string());
        }

        if !validation_errors.is_empty() {
            errors.set(validation_errors);
            loading.set(false);
            return;
        }

        // Simulate API call - in real usage, this would call an actual API
        // For now, just reset loading state immediately
        loading.set(false);
    };

    rsx! {
        div { class: "example-form",
            h2 { "表单组件示例" }

            ValidationFeedback { errors: errors() }

            Input {
                label: "用户名".to_string(),
                value: username(),
                placeholder: "请输入用户名".to_string(),
                required: true,
                oninput: move |value| username.set(value)
            }

            Input {
                label: "密码".to_string(),
                input_type: "password".to_string(),
                value: password(),
                placeholder: "请输入密码".to_string(),
                required: true,
                error: if password().len() > 0 && password().len() < 6 {
                    Some("密码至少需要6个字符".to_string())
                } else {
                    None
                },
                oninput: move |value| password.set(value)
            }

            TextArea {
                label: "个人简介".to_string(),
                value: bio(),
                placeholder: "请输入个人简介".to_string(),
                rows: 4.0,
                oninput: move |value| bio.set(value)
            }

            Select {
                label: "角色".to_string(),
                value: role(),
                options: vec![
                    ("user".to_string(), "普通用户".to_string(), None),
                    ("admin".to_string(), "管理员".to_string(), None),
                    ("moderator".to_string(), "版主".to_string(), None),
                ],
                onchange: move |value| role.set(value)
            }

            Checkbox {
                label: "我同意服务条款".to_string(),
                checked: agree_terms(),
                onchange: move |checked| agree_terms.set(checked)
            }

            div { class: "form-actions",
                Button {
                    variant: "primary".to_string(),
                    button_type: "submit".to_string(),
                    loading: loading(),
                    onclick: handle_submit,
                    "提交"
                }
                Button {
                    variant: "secondary".to_string(),
                    onclick: move |_| {
                        username.set(String::new());
                        password.set(String::new());
                        bio.set(String::new());
                        role.set("user".to_string());
                        agree_terms.set(false);
                        errors.set(Vec::new());
                    },
                    "重置"
                }
            }
        }
    }
}

/// Example demonstrating feedback components
#[component]
pub fn ExampleFeedback() -> Element {
    rsx! {
        div { class: "example-feedback",
            h2 { "反馈组件示例" }

            ErrorMessage { message: "这是一个错误消息".to_string() }
            SuccessMessage { message: "操作成功完成".to_string() }
            WarningMessage { message: "这是一个警告消息".to_string() }
            InfoMessage { message: "这是一个信息提示".to_string() }

            Alert {
                title: Some("重要通知".to_string()),
                message: "系统将在今晚进行维护".to_string(),
                alert_type: "warning".to_string(),
                dismissible: true,
                ondismiss: move |_| {}
            }

            ValidationFeedback {
                errors: vec![
                    "用户名不能为空".to_string(),
                    "密码格式不正确".to_string(),
                ],
                warnings: vec![
                    "建议使用更强的密码".to_string(),
                ]
            }
        }
    }
}

/// Example demonstrating loading components
#[component]
pub fn ExampleLoading() -> Element {
    let mut progress = use_signal(|| 0.0);
    let mut loading = use_signal(|| false);

    rsx! {
        div { class: "example-loading",
            h2 { "加载组件示例" }

            div { class: "example-section",
                h3 { "Spinner" }
                Spinner { size: "small".to_string() }
                Spinner { size: "medium".to_string(), message: Some("加载中...".to_string()) }
                Spinner { size: "large".to_string() }
            }

            div { class: "example-section",
                h3 { "Progress Bar" }
                ProgressBar {
                    value: progress(),
                    label: Some("上传进度".to_string()),
                    variant: "primary".to_string()
                }
                Button {
                    onclick: move |_| {
                        let current = progress();
                        if current < 100.0 {
                            progress.set(current + 10.0);
                        } else {
                            progress.set(0.0);
                        }
                    },
                    "增加进度"
                }
            }

            div { class: "example-section",
                h3 { "Skeleton" }
                Skeleton { variant: "text".to_string(), lines: 3 }
                Skeleton { variant: "circle".to_string(), width: "50px".to_string(), height: "50px".to_string() }
                Skeleton { variant: "rectangle".to_string(), width: "100%".to_string(), height: "100px".to_string() }
            }

            div { class: "example-section",
                h3 { "Progress Steps" }
                ProgressSteps {
                    current_step: 1,
                    steps: vec![
                        "选择配置".to_string(),
                        "验证设置".to_string(),
                        "保存配置".to_string(),
                    ]
                }
            }

            div { class: "example-section",
                h3 { "Loading Overlay" }
                Button {
                    onclick: move |_| {
                        loading.set(!loading());
                    },
                    if loading() { "隐藏加载遮罩" } else { "显示加载遮罩" }
                }
                LoadingOverlay { visible: loading(), message: "处理中，请稍候...".to_string() }
            }
        }
    }
}

/// Complete example page showing all components
#[component]
pub fn ComponentsShowcase() -> Element {
    rsx! {
        div { class: "components-showcase",
            h1 { "UI组件展示" }
            ExampleForm {}
            ExampleFeedback {}
            ExampleLoading {}
        }
    }
}
