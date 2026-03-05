//! Server commands page component

use dioxus::prelude::*;
use crate::components::loading::Spinner;
use crate::components::feedback::ErrorMessage;

/// Server command information
#[derive(Clone, Debug, PartialEq)]
struct ServerCommand {
    id: String,
    name: String,
    description: String,
    category: CommandCategory,
    requires_confirmation: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum CommandCategory {
    Maintenance,
    Database,
    Federation,
    Media,
    Users,
}

/// Scheduled command information
#[derive(Clone, Debug, PartialEq)]
struct ScheduledCommand {
    id: String,
    command_id: String,
    command_name: String,
    schedule_type: ScheduleType,
    next_run: u64,
    last_run: Option<u64>,
    is_active: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum ScheduleType {
    Once,
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

/// Server commands manager component
#[component]
pub fn ServerCommandsPage() -> Element {
    // State management
    let mut commands = use_signal(|| Vec::<ServerCommand>::new());
    let scheduled_commands = use_signal(|| Vec::<ScheduledCommand>::new());
    let mut loading = use_signal(|| false);
    let error = use_signal(|| None::<String>);
    let mut active_tab = use_signal(|| CommandTab::Available);
    let mut show_execute_dialog = use_signal(|| false);
    let mut show_schedule_dialog = use_signal(|| false);
    let mut selected_command = use_signal(|| None::<ServerCommand>);
    
    // Load commands effect (placeholder - will be implemented in Phase 4)
    use_effect(move || {
        // TODO: Implement actual API call in Phase 4
        // Mock data for UI testing
        let mock_commands = vec![
            ServerCommand {
                id: "purge_remote_media".to_string(),
                name: "清理远程媒体".to_string(),
                description: "删除指定时间之前的远程媒体文件".to_string(),
                category: CommandCategory::Media,
                requires_confirmation: true,
            },
            ServerCommand {
                id: "vacuum_database".to_string(),
                name: "数据库清理".to_string(),
                description: "优化数据库并回收空间".to_string(),
                category: CommandCategory::Database,
                requires_confirmation: true,
            },
            ServerCommand {
                id: "refresh_federation".to_string(),
                name: "刷新联邦连接".to_string(),
                description: "重新建立所有联邦连接".to_string(),
                category: CommandCategory::Federation,
                requires_confirmation: false,
            },
        ];
        commands.set(mock_commands);
        loading.set(false);
    });

    rsx! {
        div { class: "space-y-6",
            // Header
            div {
                h2 { class: "text-2xl font-bold text-gray-900", "服务器命令" }
                p { class: "mt-1 text-sm text-gray-500", "执行服务器维护和管理命令" }
            }

            // Tab navigation
            div { class: "bg-white shadow rounded-lg",
                div { class: "border-b border-gray-200",
                    nav { class: "flex -mb-px",
                        TabButton {
                            label: "可用命令",
                            active: active_tab() == CommandTab::Available,
                            onclick: move |_| active_tab.set(CommandTab::Available)
                        }
                        TabButton {
                            label: "定时命令",
                            active: active_tab() == CommandTab::Scheduled,
                            onclick: move |_| active_tab.set(CommandTab::Scheduled)
                        }
                    }
                }

                // Tab content
                div { class: "p-6",
                    if loading() {
                        div { class: "py-12 text-center",
                            Spinner { size: "large".to_string(), message: Some("加载命令列表...".to_string()) }
                        }
                    } else if let Some(err) = error() {
                        ErrorMessage { message: err }
                    } else {
                        match active_tab() {
                            CommandTab::Available => rsx! {
                                AvailableCommandsTab {
                                    commands: commands(),
                                    on_execute: move |cmd: ServerCommand| {
                                        selected_command.set(Some(cmd));
                                        show_execute_dialog.set(true);
                                    },
                                    on_schedule: move |cmd: ServerCommand| {
                                        selected_command.set(Some(cmd));
                                        show_schedule_dialog.set(true);
                                    }
                                }
                            },
                            CommandTab::Scheduled => rsx! {
                                ScheduledCommandsTab {
                                    scheduled_commands: scheduled_commands()
                                }
                            },
                        }
                    }
                }
            }

            // Execute command dialog
            if show_execute_dialog() {
                if let Some(cmd) = selected_command() {
                    ExecuteCommandDialog {
                        command: cmd,
                        on_execute: move |_| {
                            // TODO: Implement execute in Phase 4
                            show_execute_dialog.set(false);
                        },
                        on_cancel: move |_| {
                            show_execute_dialog.set(false);
                        }
                    }
                }
            }

            // Schedule command dialog
            if show_schedule_dialog() {
                if let Some(cmd) = selected_command() {
                    ScheduleCommandDialog {
                        command: cmd,
                        on_schedule: move |_| {
                            // TODO: Implement schedule in Phase 4
                            show_schedule_dialog.set(false);
                        },
                        on_cancel: move |_| {
                            show_schedule_dialog.set(false);
                        }
                    }
                }
            }
        }
    }
}

/// Command tab enum
#[derive(Clone, Copy, Debug, PartialEq)]
enum CommandTab {
    Available,
    Scheduled,
}

/// Tab button component
#[component]
fn TabButton(
    label: String,
    active: bool,
    onclick: EventHandler<()>,
) -> Element {
    let base_class = "px-6 py-3 border-b-2 font-medium text-sm";
    let active_class = if active {
        "border-blue-500 text-blue-600"
    } else {
        "border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300"
    };

    rsx! {
        button {
            class: "{base_class} {active_class}",
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// Available commands tab component
#[component]
fn AvailableCommandsTab(
    commands: Vec<ServerCommand>,
    on_execute: EventHandler<ServerCommand>,
    on_schedule: EventHandler<ServerCommand>,
) -> Element {
    rsx! {
        div { class: "space-y-4",
            if commands.is_empty() {
                div { class: "text-center py-12",
                    p { class: "text-gray-500", "暂无可用命令" }
                }
            } else {
                for cmd in commands {
                    CommandCard {
                        key: "{cmd.id}",
                        command: cmd.clone(),
                        on_execute: move |c: ServerCommand| on_execute.call(c),
                        on_schedule: move |c: ServerCommand| on_schedule.call(c)
                    }
                }
            }
        }
    }
}

/// Command card component
#[component]
fn CommandCard(
    command: ServerCommand,
    on_execute: EventHandler<ServerCommand>,
    on_schedule: EventHandler<ServerCommand>,
) -> Element {
    let category_badge = match command.category {
        CommandCategory::Maintenance => ("bg-blue-100 text-blue-800", "🔧 维护"),
        CommandCategory::Database => ("bg-purple-100 text-purple-800", "🗄️ 数据库"),
        CommandCategory::Federation => ("bg-green-100 text-green-800", "🌐 联邦"),
        CommandCategory::Media => ("bg-yellow-100 text-yellow-800", "🖼️ 媒体"),
        CommandCategory::Users => ("bg-pink-100 text-pink-800", "👥 用户"),
    };
    
    let cmd_for_schedule = command.clone();
    let cmd_for_execute = command.clone();

    rsx! {
        div { class: "border border-gray-200 rounded-lg p-4 hover:border-blue-300 transition-colors",
            div { class: "flex items-start justify-between",
                div { class: "flex-1",
                    div { class: "flex items-center gap-2 mb-2",
                        h3 { class: "text-lg font-medium text-gray-900", "{command.name}" }
                        span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium {category_badge.0}",
                            "{category_badge.1}"
                        }
                        if command.requires_confirmation {
                            span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800",
                                "⚠️ 需确认"
                            }
                        }
                    }
                    p { class: "text-sm text-gray-600", "{command.description}" }
                }
                div { class: "flex gap-2 ml-4",
                    button {
                        class: "inline-flex items-center px-3 py-2 border border-gray-300 text-sm font-medium rounded-md text-gray-700 bg-white hover:bg-gray-50",
                        onclick: move |_| on_schedule.call(cmd_for_schedule.clone()),
                        "⏰ 定时"
                    }
                    button {
                        class: "inline-flex items-center px-3 py-2 border border-transparent text-sm font-medium rounded-md text-white bg-blue-600 hover:bg-blue-700",
                        onclick: move |_| on_execute.call(cmd_for_execute.clone()),
                        "▶️ 执行"
                    }
                }
            }
        }
    }
}

/// Scheduled commands tab component
#[component]
fn ScheduledCommandsTab(scheduled_commands: Vec<ScheduledCommand>) -> Element {
    rsx! {
        div { class: "space-y-4",
            if scheduled_commands.is_empty() {
                div { class: "text-center py-12",
                    div { class: "text-gray-400 text-5xl mb-4", "⏰" }
                    p { class: "text-gray-500 text-lg", "暂无定时命令" }
                    p { class: "text-gray-400 text-sm mt-2", "在"可用命令"标签页中点击"定时"按钮创建定时命令" }
                }
            } else {
                div { class: "overflow-x-auto",
                    table { class: "min-w-full divide-y divide-gray-200",
                        thead { class: "bg-gray-50",
                            tr {
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "命令" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "类型" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "下次运行" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "上次运行" }
                                th { class: "px-6 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider", "状态" }
                                th { class: "px-6 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider", "操作" }
                            }
                        }
                        tbody { class: "bg-white divide-y divide-gray-200",
                            for scheduled in scheduled_commands {
                                ScheduledCommandRow {
                                    key: "{scheduled.id}",
                                    scheduled: scheduled.clone()
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Scheduled command row component
#[component]
fn ScheduledCommandRow(scheduled: ScheduledCommand) -> Element {
    let schedule_type_text = match scheduled.schedule_type {
        ScheduleType::Once => "一次性",
        ScheduleType::Hourly => "每小时",
        ScheduleType::Daily => "每天",
        ScheduleType::Weekly => "每周",
        ScheduleType::Monthly => "每月",
    };

    rsx! {
        tr { class: "hover:bg-gray-50",
            td { class: "px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900",
                "{scheduled.command_name}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{schedule_type_text}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                "{format_timestamp(scheduled.next_run)}"
            }
            td { class: "px-6 py-4 whitespace-nowrap text-sm text-gray-500",
                if let Some(last_run) = scheduled.last_run {
                    "{format_timestamp(last_run)}"
                } else {
                    "-"
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap",
                if scheduled.is_active {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800",
                        "🟢 活跃"
                    }
                } else {
                    span { class: "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium bg-gray-100 text-gray-800",
                        "⚪ 暂停"
                    }
                }
            }
            td { class: "px-6 py-4 whitespace-nowrap text-right text-sm font-medium",
                div { class: "flex justify-end gap-2",
                    button {
                        class: "text-blue-600 hover:text-blue-900",
                        onclick: move |_| {
                            // TODO: Implement edit
                        },
                        "编辑"
                    }
                    button {
                        class: "text-red-600 hover:text-red-900",
                        onclick: move |_| {
                            // TODO: Implement delete
                        },
                        "删除"
                    }
                }
            }
        }
    }
}

/// Execute command dialog component
#[component]
fn ExecuteCommandDialog(
    command: ServerCommand,
    on_execute: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "fixed inset-0 bg-gray-500 bg-opacity-75 flex items-center justify-center z-50 p-4",
            div { class: "bg-white rounded-lg shadow-xl max-w-md w-full",
                div { class: "px-6 py-4 border-b border-gray-200",
                    h3 { class: "text-lg font-medium text-gray-900",
                        "执行命令"
                    }
                }
                div { class: "px-6 py-4",
                    p { class: "text-sm text-gray-700 mb-4",
                        "确定要执行以下命令吗？"
                    }
                    div { class: "bg-gray-50 rounded-md p-4",
                        p { class: "text-sm font-medium text-gray-900", "{command.name}" }
                        p { class: "text-xs text-gray-600 mt-1", "{command.description}" }
                    }
                    if command.requires_confirmation {
                        div { class: "mt-4 bg-yellow-50 border border-yellow-200 rounded-md p-3",
                            p { class: "text-xs text-yellow-800",
                                "⚠️ 此操作需要确认。请确保您了解此命令的影响。"
                            }
                        }
                    }
                }
                div { class: "px-6 py-4 bg-gray-50 flex justify-end gap-3 rounded-b-lg border-t border-gray-200",
                    button {
                        class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50",
                        onclick: move |_| on_cancel.call(()),
                        "取消"
                    }
                    button {
                        class: "px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700",
                        onclick: move |_| on_execute.call(()),
                        "执行"
                    }
                }
            }
        }
    }
}

/// Schedule command dialog component
#[component]
fn ScheduleCommandDialog(
    command: ServerCommand,
    on_schedule: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut schedule_type = use_signal(|| ScheduleType::Once);

    rsx! {
        div { class: "fixed inset-0 bg-gray-500 bg-opacity-75 flex items-center justify-center z-50 p-4",
            div { class: "bg-white rounded-lg shadow-xl max-w-md w-full",
                div { class: "px-6 py-4 border-b border-gray-200",
                    h3 { class: "text-lg font-medium text-gray-900",
                        "定时执行命令"
                    }
                }
                div { class: "px-6 py-4",
                    div { class: "space-y-4",
                        div { class: "bg-gray-50 rounded-md p-4",
                            p { class: "text-sm font-medium text-gray-900", "{command.name}" }
                            p { class: "text-xs text-gray-600 mt-1", "{command.description}" }
                        }
                        
                        div {
                            label { class: "block text-sm font-medium text-gray-700 mb-2",
                                "执行频率"
                            }
                            select {
                                class: "w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-blue-500 focus:border-blue-500",
                                onchange: move |evt| {
                                    schedule_type.set(match evt.value().as_str() {
                                        "once" => ScheduleType::Once,
                                        "hourly" => ScheduleType::Hourly,
                                        "daily" => ScheduleType::Daily,
                                        "weekly" => ScheduleType::Weekly,
                                        "monthly" => ScheduleType::Monthly,
                                        _ => ScheduleType::Once,
                                    });
                                },
                                option { value: "once", "一次性" }
                                option { value: "hourly", "每小时" }
                                option { value: "daily", "每天" }
                                option { value: "weekly", "每周" }
                                option { value: "monthly", "每月" }
                            }
                        }
                    }
                }
                div { class: "px-6 py-4 bg-gray-50 flex justify-end gap-3 rounded-b-lg border-t border-gray-200",
                    button {
                        class: "px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50",
                        onclick: move |_| on_cancel.call(()),
                        "取消"
                    }
                    button {
                        class: "px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700",
                        onclick: move |_| on_schedule.call(()),
                        "创建定时任务"
                    }
                }
            }
        }
    }
}

/// Format Unix timestamp to readable date string
fn format_timestamp(ts: u64) -> String {
    use chrono::{Utc, TimeZone};
    
    let dt = Utc.timestamp_opt(ts as i64, 0).single();
    match dt {
        Some(datetime) => datetime.format("%Y-%m-%d %H:%M").to_string(),
        None => "无效时间".to_string(),
    }
}
