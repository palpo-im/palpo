//! Admin dashboard page component

use dioxus::prelude::*;
use dioxus_router::Link;
use crate::hooks::use_auth;
use crate::app::Route;

/// Admin dashboard component
#[component]
pub fn AdminDashboard() -> Element {
    let auth_context = use_auth();

    rsx! {
        div { class: "space-y-6",
            // Welcome section
            div { class: "bg-white overflow-hidden shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    div { class: "sm:flex sm:items-center sm:justify-between",
                        div { class: "sm:flex sm:space-x-5",
                            div { class: "mt-4 text-center sm:mt-0 sm:pt-1 sm:text-left",
                                p { class: "text-xl font-bold text-gray-900 sm:text-2xl",
                                    "欢迎回来"
                                    if let Some(user) = auth_context.current_user() {
                                        ", {user.username}"
                                    }
                                }
                                p { class: "text-sm font-medium text-gray-600",
                                    "Palpo Matrix 服务器管理界面"
                                }
                            }
                        }
                    }
                }
            }

            // Quick stats
            div { class: "grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-4",
                // Server status card
                div { class: "bg-white overflow-hidden shadow rounded-lg",
                    div { class: "p-5",
                        div { class: "flex items-center",
                            div { class: "flex-shrink-0",
                                div { class: "w-8 h-8 bg-green-500 rounded-md flex items-center justify-center",
                                    span { class: "text-white text-sm", "✓" }
                                }
                            }
                            div { class: "ml-5 w-0 flex-1",
                                dl {
                                    dt { class: "text-sm font-medium text-gray-500 truncate",
                                        "服务器状态"
                                    }
                                    dd { class: "text-lg font-medium text-gray-900",
                                        "运行中"
                                    }
                                }
                            }
                        }
                    }
                }

                // Users card
                div { class: "bg-white overflow-hidden shadow rounded-lg",
                    div { class: "p-5",
                        div { class: "flex items-center",
                            div { class: "flex-shrink-0",
                                div { class: "w-8 h-8 bg-blue-500 rounded-md flex items-center justify-center",
                                    span { class: "text-white text-sm", "👥" }
                                }
                            }
                            div { class: "ml-5 w-0 flex-1",
                                dl {
                                    dt { class: "text-sm font-medium text-gray-500 truncate",
                                        "注册用户"
                                    }
                                    dd { class: "text-lg font-medium text-gray-900",
                                        "加载中..."
                                    }
                                }
                            }
                        }
                    }
                }

                // Rooms card
                div { class: "bg-white overflow-hidden shadow rounded-lg",
                    div { class: "p-5",
                        div { class: "flex items-center",
                            div { class: "flex-shrink-0",
                                div { class: "w-8 h-8 bg-purple-500 rounded-md flex items-center justify-center",
                                    span { class: "text-white text-sm", "🏠" }
                                }
                            }
                            div { class: "ml-5 w-0 flex-1",
                                dl {
                                    dt { class: "text-sm font-medium text-gray-500 truncate",
                                        "房间数量"
                                    }
                                    dd { class: "text-lg font-medium text-gray-900",
                                        "加载中..."
                                    }
                                }
                            }
                        }
                    }
                }

                // Federation card
                div { class: "bg-white overflow-hidden shadow rounded-lg",
                    div { class: "p-5",
                        div { class: "flex items-center",
                            div { class: "flex-shrink-0",
                                div { class: "w-8 h-8 bg-indigo-500 rounded-md flex items-center justify-center",
                                    span { class: "text-white text-sm", "🌐" }
                                }
                            }
                            div { class: "ml-5 w-0 flex-1",
                                dl {
                                    dt { class: "text-sm font-medium text-gray-500 truncate",
                                        "联邦服务器"
                                    }
                                    dd { class: "text-lg font-medium text-gray-900",
                                        "加载中..."
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Quick actions
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900 mb-4",
                        "快速操作"
                    }
                    div { class: "grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3",
                        // Server management
                        div { class: "relative rounded-lg border border-gray-300 bg-white px-6 py-5 shadow-sm flex items-center space-x-3 hover:border-gray-400 focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-blue-500",
                            div { class: "flex-shrink-0",
                                span { class: "text-2xl", "🎛️" }
                            }
                            div { class: "flex-1 min-w-0",
                                Link {
                                    to: Route::ServerControl {},
                                    class: "focus:outline-none",
                                    span { class: "absolute inset-0", "aria-hidden": "true" }
                                    p { class: "text-sm font-medium text-gray-900", "服务器管理" }
                                    p { class: "text-sm text-gray-500", "管理服务器配置与状态" }
                                }
                            }
                        }

                        // User management
                        div { class: "relative rounded-lg border border-gray-300 bg-white px-6 py-5 shadow-sm flex items-center space-x-3 hover:border-gray-400 focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-blue-500",
                            div { class: "flex-shrink-0",
                                span { class: "text-2xl", "👥" }
                            }
                            div { class: "flex-1 min-w-0",
                                Link {
                                    to: Route::Users {},
                                    class: "focus:outline-none",
                                    span { class: "absolute inset-0", "aria-hidden": "true" }
                                    p { class: "text-sm font-medium text-gray-900", "用户管理" }
                                    p { class: "text-sm text-gray-500", "管理用户账户" }
                                }
                            }
                        }

                        // Room management
                        div { class: "relative rounded-lg border border-gray-300 bg-white px-6 py-5 shadow-sm flex items-center space-x-3 hover:border-gray-400 focus-within:ring-2 focus-within:ring-offset-2 focus-within:ring-blue-500",
                            div { class: "flex-shrink-0",
                                span { class: "text-2xl", "🏠" }
                            }
                            div { class: "flex-1 min-w-0",
                                Link {
                                    to: Route::Rooms {},
                                    class: "focus:outline-none",
                                    span { class: "absolute inset-0", "aria-hidden": "true" }
                                    p { class: "text-sm font-medium text-gray-900", "房间管理" }
                                    p { class: "text-sm text-gray-500", "管理聊天房间" }
                                }
                            }
                        }
                    }
                }
            }

            // Recent activity (placeholder)
            div { class: "bg-white shadow rounded-lg",
                div { class: "px-4 py-5 sm:p-6",
                    h3 { class: "text-lg leading-6 font-medium text-gray-900 mb-4",
                        "最近活动"
                    }
                    div { class: "text-center py-8",
                        p { class: "text-gray-500", "暂无活动记录" }
                    }
                }
            }
        }
    }
}