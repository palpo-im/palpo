//! Main application component

use dioxus::prelude::*;

#[component]
pub fn App() -> Element {
    rsx! {
        div { class: "min-h-screen bg-gray-100",
            h1 { class: "text-3xl font-bold text-center py-8",
                "Palpo 管理界面"
            }
            p { class: "text-center text-gray-600",
                "现代化的 Matrix 服务器管理界面"
            }
        }
    }
}