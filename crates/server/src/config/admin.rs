use serde::Deserialize;

use crate::core::serde::default_true;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "admin")]
#[derive(Clone, Debug, Deserialize)]
pub struct AdminConfig {
    /// Controls whether admin room notices like account registrations, password
    /// changes, account deactivations, room directory publications, etc will be
    /// sent to the admin room. Update notices and normal admin command
    /// responses will still be sent.
    #[serde(default = "default_true")]
    pub room_notices: bool,

    /// Allow admins to enter commands in rooms other than "#admins" (admin
    /// room) by prefixing your message with "\!admin" or "\\!admin" followed up
    /// a normal palpo admin command. The reply will be publicly visible to
    /// the room, originating from the sender.
    ///
    /// example: \\!admin debug ping example.com
    #[serde(default = "default_true")]
    pub escape_commands: bool,

    /// Automatically activate the palpo admin room console / CLI on
    /// startup. This option can also be enabled with `--console` palpo
    /// argument.
    #[serde(default)]
    pub console_automatic: bool,

    #[allow(clippy::doc_link_with_quotes)]
    /// List of admin commands to execute on startup.
    ///
    /// This option can also be configured with the `--execute` palpo
    /// argument and can take standard shell commands and environment variables
    ///
    /// For example: `./palpo --execute "server admin-notice palpo has
    /// started up at $(date)"`
    ///
    /// example: admin_execute = ["debug ping example.com", "debug echo hi"]`
    ///
    /// default: []
    #[serde(default)]
    pub startup_execute: Vec<String>,

    /// Ignore errors in startup commands.
    ///
    /// If false, palpo will error and fail to start if an admin execute
    /// command (`--execute` / `admin_execute`) fails.
    #[serde(default)]
    pub execute_errors_ignore: bool,

    /// List of admin commands to execute on SIGUSR2.
    ///
    /// Similar to admin_execute, but these commands are executed when the
    /// server receives SIGUSR2 on supporting platforms.
    ///
    /// default: []
    #[serde(default)]
    pub signal_execute: Vec<String>,

    /// Controls the max log level for admin command log captures (logs
    /// generated from running admin commands). Defaults to "info" on release
    /// builds, else "debug" on debug builds.
    ///
    /// default: "info"
    #[serde(default = "default_log_capture")]
    pub log_capture: String,

    /// The default room tag to apply on the admin room.
    ///
    /// On some clients like Element, the room tag "m.server_notice" is a
    /// special pinned room at the very bottom of your room list. The palpo
    /// admin room can be pinned here so you always have an easy-to-access
    /// shortcut dedicated to your admin room.
    ///
    /// default: "m.server_notice"
    #[serde(default = "default_room_tag")]
    pub room_tag: String,

    /// Shared secret for MAS (Matrix Authentication Service) endpoints.
    /// When set, enables the `_synapse/mas/` endpoints which use this
    /// secret as a Bearer token for authentication.
    #[serde(default)]
    pub mas_secret: Option<String>,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            room_notices: true,
            escape_commands: true,
            console_automatic: false,
            startup_execute: Vec::new(),
            execute_errors_ignore: false,
            signal_execute: Vec::new(),
            log_capture: default_log_capture(),
            room_tag: default_room_tag(),
            mas_secret: None,
        }
    }
}

fn default_log_capture() -> String {
    cfg!(debug_assertions)
        .then_some("debug")
        .unwrap_or("info")
        .to_owned()
}
fn default_room_tag() -> String {
    "m.server_notice".to_owned()
}
