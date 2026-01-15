mod cmd;
use std::path::PathBuf;

use clap::Subcommand;
use cmd::*;

use crate::AppResult;
use crate::macros::admin_command_dispatch;

#[admin_command_dispatch]
#[derive(Debug, Subcommand)]
pub(crate) enum ServerCommand {
    // /// - Time elapsed since startup
    // Uptime,
    /// - Show configuration values
    ShowConfig,

    /// - Reload configuration values
    ReloadConfig { path: Option<PathBuf> },

    /// - List the features built into the server
    ListFeatures {
        #[arg(short, long)]
        available: bool,

        #[arg(short, long)]
        enabled: bool,

        #[arg(short, long)]
        comma: bool,
    },

    /// - Send a message to the admin room.
    AdminNotice { message: Vec<String> },

    /// - Hot-reload the server
    #[clap(alias = "reload")]
    ReloadMods,

    /// - Restart the server
    Restart {
        #[arg(short, long)]
        force: bool,
    },

    /// - Shutdown the server
    Shutdown,
}
