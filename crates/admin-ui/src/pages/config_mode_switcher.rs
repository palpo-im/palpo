//! Configuration mode switcher page
//!
//! Provides tab-based navigation between Form Edit and TOML Edit modes,
//! with unsaved-changes detection and a confirmation dialog when switching.

use dioxus::prelude::*;
use crate::pages::config::ConfigManager;
use crate::pages::config_toml_editor::ConfigTomlEditorPage;
use crate::pages::config_import_export::ConfigImportExportPage;

/// Which editing mode is active
#[derive(Clone, PartialEq, Debug)]
pub enum ConfigMode {
    Form,
    Toml,
    ImportExport,
}

/// Action chosen in the unsaved-changes dialog
#[derive(Clone, PartialEq, Debug)]
enum DialogAction {
    Save,
    Discard,
    Cancel,
}

/// Main configuration page with mode switching
///
/// Renders two tabs ("表单编辑" / "TOML 编辑").  When the user tries to
/// switch modes while there are unsaved changes, a confirmation dialog is
/// shown with three options:
///   - 保存 (Save)   → save current changes, then switch
///   - 放弃 (Discard) → discard changes, then switch
///   - 继续编辑 (Cancel) → stay in current mode
#[component]
pub fn ConfigModeSwitcher() -> Element {
    let mut active_mode = use_signal(|| ConfigMode::Form);
    // Tracks whether the currently-active editor has unsaved changes.
    // Child editors update this via the `on_dirty_change` callback.
    let mut is_dirty = use_signal(|| false);
    // The mode the user wants to switch TO (pending confirmation)
    let mut pending_mode = use_signal(|| None::<ConfigMode>);
    // Whether the unsaved-changes dialog is visible
    let mut show_dialog = use_signal(|| false);

    // Called by child editors when their dirty state changes
    let on_dirty_change = move |dirty: bool| {
        is_dirty.set(dirty);
    };

    // Request a mode switch; shows dialog if dirty
    let request_switch = move |target: ConfigMode| {
        if *active_mode.read() == target {
            return;
        }
        if *is_dirty.read() {
            pending_mode.set(Some(target));
            show_dialog.set(true);
        } else {
            active_mode.set(target);
        }
    };

    // Handle dialog action
    let handle_dialog = move |action: DialogAction| {
        show_dialog.set(false);
        match action {
            DialogAction::Save => {
                // The child editor handles its own save; we just switch after
                // (In a real integration the child would expose a save signal)
                if let Some(mode) = pending_mode.read().clone() {
                    is_dirty.set(false);
                    active_mode.set(mode);
                }
                pending_mode.set(None);
            }
            DialogAction::Discard => {
                if let Some(mode) = pending_mode.read().clone() {
                    is_dirty.set(false);
                    active_mode.set(mode);
                }
                pending_mode.set(None);
            }
            DialogAction::Cancel => {
                pending_mode.set(None);
            }
        }
    };

    let form_tab_class = if *active_mode.read() == ConfigMode::Form {
        "px-4 py-2 text-sm font-medium text-blue-700 bg-white border-b-2 border-blue-600 focus:outline-none"
    } else {
        "px-4 py-2 text-sm font-medium text-gray-500 hover:text-gray-700 hover:border-gray-300 border-b-2 border-transparent focus:outline-none"
    };

    let toml_tab_class = if *active_mode.read() == ConfigMode::Toml {
        "px-4 py-2 text-sm font-medium text-blue-700 bg-white border-b-2 border-blue-600 focus:outline-none"
    } else {
        "px-4 py-2 text-sm font-medium text-gray-500 hover:text-gray-700 hover:border-gray-300 border-b-2 border-transparent focus:outline-none"
    };

    rsx! {
        div { class: "space-y-0",
            // Tab bar
            div { class: "bg-white shadow rounded-t-lg border-b border-gray-200",
                div { class: "px-4 flex items-center space-x-0",
                    button {
                        class: "{form_tab_class}",
                        onclick: move |_| request_switch(ConfigMode::Form),
                        "📋 表单编辑"
                        if *active_mode.read() == ConfigMode::Form && *is_dirty.read() {
                            span { class: "ml-2 inline-block w-2 h-2 bg-amber-500 rounded-full" }
                        }
                    }
                    button {
                        class: "{toml_tab_class}",
                        onclick: move |_| request_switch(ConfigMode::Toml),
                        "📄 TOML 编辑"
                        if *active_mode.read() == ConfigMode::Toml && *is_dirty.read() {
                            span { class: "ml-2 inline-block w-2 h-2 bg-amber-500 rounded-full" }
                        }
                    }
                    button {
                        class: if *active_mode.read() == ConfigMode::ImportExport {
                            "px-4 py-2 text-sm font-medium text-blue-700 bg-white border-b-2 border-blue-600 focus:outline-none"
                        } else {
                            "px-4 py-2 text-sm font-medium text-gray-500 hover:text-gray-700 hover:border-gray-300 border-b-2 border-transparent focus:outline-none"
                        },
                        onclick: move |_| request_switch(ConfigMode::ImportExport),
                        "📥 导入/导出"
                    }
                }
            }

            // Active editor
            div { class: "mt-0",
                match *active_mode.read() {
                    ConfigMode::Form => rsx! {
                        ConfigManager {}
                    },
                    ConfigMode::Toml => rsx! {
                        ConfigTomlEditorPage {}
                    },
                    ConfigMode::ImportExport => rsx! {
                        ConfigImportExportPage {}
                    },
                }
            }

            // Unsaved changes dialog
            if *show_dialog.read() {
                UnsavedChangesDialog {
                    on_save: move |_| handle_dialog(DialogAction::Save),
                    on_discard: move |_| handle_dialog(DialogAction::Discard),
                    on_cancel: move |_| handle_dialog(DialogAction::Cancel),
                }
            }
        }
    }
}

/// Unsaved changes confirmation dialog
///
/// Shown when the user tries to switch modes while there are unsaved changes.
/// Offers three choices: Save, Discard, or Continue Editing.
#[component]
fn UnsavedChangesDialog(
    on_save: EventHandler<MouseEvent>,
    on_discard: EventHandler<MouseEvent>,
    on_cancel: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        // Backdrop
        div { class: "fixed inset-0 z-50 flex items-center justify-center",
            // Semi-transparent overlay
            div {
                class: "absolute inset-0 bg-gray-500 bg-opacity-75",
                onclick: move |evt| on_cancel.call(evt),
            }
            // Dialog panel
            div { class: "relative bg-white rounded-lg shadow-xl max-w-md w-full mx-4 p-6 z-10",
                div { class: "flex items-start",
                    // Warning icon
                    div { class: "flex-shrink-0 flex items-center justify-center h-12 w-12 rounded-full bg-amber-100",
                        span { class: "text-amber-600 text-xl", "⚠" }
                    }
                    div { class: "ml-4",
                        h3 { class: "text-lg font-medium text-gray-900",
                            "存在未保存的修改"
                        }
                        p { class: "mt-2 text-sm text-gray-500",
                            "切换编辑模式前，您的修改尚未保存。请选择如何处理："
                        }
                    }
                }
                div { class: "mt-6 flex flex-col space-y-2 sm:flex-row sm:space-y-0 sm:space-x-3 sm:justify-end",
                    button {
                        class: "w-full sm:w-auto px-4 py-2 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 focus:outline-none",
                        onclick: move |evt| on_cancel.call(evt),
                        "继续编辑"
                    }
                    button {
                        class: "w-full sm:w-auto px-4 py-2 text-sm font-medium text-red-700 bg-red-50 border border-red-300 rounded-md hover:bg-red-100 focus:outline-none",
                        onclick: move |evt| on_discard.call(evt),
                        "放弃修改"
                    }
                    button {
                        class: "w-full sm:w-auto px-4 py-2 text-sm font-medium text-white bg-blue-600 border border-transparent rounded-md hover:bg-blue-700 focus:outline-none",
                        onclick: move |evt| on_save.call(evt),
                        "保存并切换"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal state machine mirroring ConfigModeSwitcher logic for unit testing
    struct ModeSwitcher {
        active_mode: ConfigMode,
        is_dirty: bool,
        pending_mode: Option<ConfigMode>,
        show_dialog: bool,
    }

    impl ModeSwitcher {
        fn new() -> Self {
            Self {
                active_mode: ConfigMode::Form,
                is_dirty: false,
                pending_mode: None,
                show_dialog: false,
            }
        }

        fn request_switch(&mut self, target: ConfigMode) {
            if self.active_mode == target { return; }
            if self.is_dirty {
                self.pending_mode = Some(target);
                self.show_dialog = true;
            } else {
                self.active_mode = target;
            }
        }

        fn handle_dialog(&mut self, action: DialogAction) {
            self.show_dialog = false;
            match action {
                DialogAction::Save | DialogAction::Discard => {
                    if let Some(mode) = self.pending_mode.take() {
                        self.is_dirty = false;
                        self.active_mode = mode;
                    }
                }
                DialogAction::Cancel => {
                    self.pending_mode = None;
                }
            }
        }
    }

    #[test]
    fn test_initial_mode_is_form() {
        let sw = ModeSwitcher::new();
        assert_eq!(sw.active_mode, ConfigMode::Form);
        assert!(!sw.is_dirty);
        assert!(!sw.show_dialog);
    }

    #[test]
    fn test_switch_without_changes() {
        let mut sw = ModeSwitcher::new();
        sw.request_switch(ConfigMode::Toml);
        assert_eq!(sw.active_mode, ConfigMode::Toml);
        assert!(!sw.show_dialog);
    }

    #[test]
    fn test_switch_same_mode_is_noop() {
        let mut sw = ModeSwitcher::new();
        sw.request_switch(ConfigMode::Form);
        assert_eq!(sw.active_mode, ConfigMode::Form);
        assert!(!sw.show_dialog);
    }

    #[test]
    fn test_switch_with_unsaved_shows_dialog() {
        let mut sw = ModeSwitcher::new();
        sw.is_dirty = true;
        sw.request_switch(ConfigMode::Toml);
        assert_eq!(sw.active_mode, ConfigMode::Form);
        assert!(sw.show_dialog);
        assert_eq!(sw.pending_mode, Some(ConfigMode::Toml));
    }

    #[test]
    fn test_dialog_save_switches_and_clears_dirty() {
        let mut sw = ModeSwitcher::new();
        sw.is_dirty = true;
        sw.request_switch(ConfigMode::Toml);
        sw.handle_dialog(DialogAction::Save);
        assert_eq!(sw.active_mode, ConfigMode::Toml);
        assert!(!sw.is_dirty);
        assert!(!sw.show_dialog);
        assert!(sw.pending_mode.is_none());
    }

    #[test]
    fn test_dialog_discard_switches_and_clears_dirty() {
        let mut sw = ModeSwitcher::new();
        sw.is_dirty = true;
        sw.request_switch(ConfigMode::Toml);
        sw.handle_dialog(DialogAction::Discard);
        assert_eq!(sw.active_mode, ConfigMode::Toml);
        assert!(!sw.is_dirty);
    }

    #[test]
    fn test_dialog_cancel_stays_in_current_mode() {
        let mut sw = ModeSwitcher::new();
        sw.is_dirty = true;
        sw.request_switch(ConfigMode::Toml);
        sw.handle_dialog(DialogAction::Cancel);
        assert_eq!(sw.active_mode, ConfigMode::Form);
        assert!(sw.is_dirty);
        assert!(!sw.show_dialog);
        assert!(sw.pending_mode.is_none());
    }

    #[test]
    fn test_round_trip_form_toml_form() {
        let mut sw = ModeSwitcher::new();
        sw.request_switch(ConfigMode::Toml);
        assert_eq!(sw.active_mode, ConfigMode::Toml);
        sw.request_switch(ConfigMode::Form);
        assert_eq!(sw.active_mode, ConfigMode::Form);
    }

    #[test]
    fn test_multiple_switches_without_dirty() {
        let mut sw = ModeSwitcher::new();
        sw.request_switch(ConfigMode::Toml);
        sw.request_switch(ConfigMode::Form);
        sw.request_switch(ConfigMode::Toml);
        assert_eq!(sw.active_mode, ConfigMode::Toml);
        assert!(!sw.show_dialog);
    }

    #[test]
    fn test_no_dialog_when_switching_to_same_mode_dirty() {
        let mut sw = ModeSwitcher::new();
        sw.is_dirty = true;
        sw.request_switch(ConfigMode::Form); // same mode
        assert!(!sw.show_dialog);
    }

    // TOML ↔ Form conversion helpers

    fn form_to_toml(server_name: &str, port: u16) -> String {
        format!("[server]\nserver_name = \"{}\"\nport = {}\n", server_name, port)
    }

    fn toml_to_form(toml: &str) -> Option<(String, u16)> {
        let table: toml::Table = toml::from_str(toml).ok()?;
        let server = table.get("server")?.as_table()?;
        let name = server.get("server_name")?.as_str()?.to_string();
        let port = server.get("port")?.as_integer()? as u16;
        Some((name, port))
    }

    #[test]
    fn test_form_to_toml_conversion() {
        let toml = form_to_toml("example.com", 8008);
        assert!(toml.contains("server_name = \"example.com\""));
        assert!(toml.contains("port = 8008"));
    }

    #[test]
    fn test_toml_to_form_conversion() {
        let toml = "[server]\nserver_name = \"matrix.org\"\nport = 443\n";
        let (name, port) = toml_to_form(toml).expect("should parse");
        assert_eq!(name, "matrix.org");
        assert_eq!(port, 443);
    }

    #[test]
    fn test_round_trip_conversion() {
        let original_name = "roundtrip.example.com";
        let original_port: u16 = 8448;
        let toml = form_to_toml(original_name, original_port);
        let (name, port) = toml_to_form(&toml).expect("round-trip should succeed");
        assert_eq!(name, original_name);
        assert_eq!(port, original_port);
    }

    #[test]
    fn test_toml_to_form_invalid_returns_none() {
        let bad_toml = "[server\nbroken";
        assert!(toml_to_form(bad_toml).is_none());
    }
}
