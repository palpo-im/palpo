//! Login page component

use dioxus::prelude::*;
use crate::hooks::use_auth;
use crate::components::ForgotPasswordModal;

/// Login page component
#[component]
pub fn LoginPage() -> Element {
    let auth_context = use_auth();
    let mut username = use_signal(|| String::new());
    let mut password = use_signal(|| String::new());
    let mut show_error = use_signal(|| false);
    let mut show_forgot_password = use_signal(|| false);

    let handle_login = {
        let auth_context = auth_context.clone();
        move |_| {
            let username_val = username.read().clone();
            let password_val = password.read().clone();
            
            if username_val.is_empty() || password_val.is_empty() {
                show_error.set(true);
                return;
            }
            
            show_error.set(false);
            auth_context.login(username_val, password_val);
        }
    };

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8",
            div { class: "max-w-md w-full space-y-8",
                div {
                    div { class: "mx-auto h-12 w-12 bg-blue-600 rounded-lg flex items-center justify-center",
                        span { class: "text-white font-bold text-xl", "P" }
                    }
                    h2 { class: "mt-6 text-center text-3xl font-extrabold text-gray-900",
                        "登录 Palpo 管理界面"
                    }
                    p { class: "mt-2 text-center text-sm text-gray-600",
                        "请使用管理员账户登录"
                    }
                }
                
                form { 
                    class: "mt-8 space-y-6",
                    onsubmit: move |evt| evt.prevent_default(),
                    
                    div { class: "rounded-md shadow-sm -space-y-px",
                        div {
                            label { 
                                r#for: "username",
                                class: "sr-only",
                                "用户名"
                            }
                            input {
                                id: "username",
                                name: "username",
                                r#type: "text",
                                required: true,
                                class: "appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-t-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm",
                                placeholder: "用户名",
                                value: "{username}",
                                oninput: move |evt| username.set(evt.value().clone())
                            }
                        }
                        div {
                            label { 
                                r#for: "password",
                                class: "sr-only",
                                "密码"
                            }
                            input {
                                id: "password",
                                name: "password",
                                r#type: "password",
                                required: true,
                                class: "appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-b-md focus:outline-none focus:ring-blue-500 focus:border-blue-500 focus:z-10 sm:text-sm",
                                placeholder: "密码",
                                value: "{password}",
                                oninput: move |evt| password.set(evt.value().clone())
                            }
                        }
                    }

                    if *show_error.read() || auth_context.auth_error().is_some() {
                        div { class: "rounded-md bg-red-50 p-4",
                            div { class: "flex",
                                div { class: "ml-3",
                                    h3 { class: "text-sm font-medium text-red-800",
                                        if let Some(error) = auth_context.auth_error() {
                                            "{error}"
                                        } else {
                                            "请填写用户名和密码"
                                        }
                                    }
                                }
                            }
                        }
                    }

                    div { class: "flex items-center justify-between",
                        div { class: "text-sm",
                            button {
                                r#type: "button",
                                onclick: move |_| show_forgot_password.set(true),
                                class: "font-medium text-blue-600 hover:text-blue-500",
                                "忘记密码？"
                            }
                        }
                    }

                    div {
                        button {
                            r#type: "button",
                            onclick: handle_login,
                            disabled: auth_context.is_authenticating(),
                            class: "group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-blue-500 disabled:opacity-50 disabled:cursor-not-allowed",
                            
                            if auth_context.is_authenticating() {
                                div { class: "animate-spin rounded-full h-4 w-4 border-b-2 border-white mr-2" }
                                "登录中..."
                            } else {
                                "登录"
                            }
                        }
                    }
                }
            }
        }
        
        // Forgot password modal
        ForgotPasswordModal {
            show: show_forgot_password(),
            onclose: move |_| show_forgot_password.set(false)
        }
    }
}