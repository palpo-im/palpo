pub(crate) mod appservice;
mod console;
// pub(crate) mod debug;
pub(crate) mod federation;
pub(crate) mod media;
pub(crate) mod room;
pub(crate) mod server;
pub(crate) mod user;
pub(crate) use console::Console;
mod utils;
pub(crate) use utils::*;
mod executor;
mod processor;

// Two-tier admin system types
pub(crate) mod types;
pub(crate) use types::{
    AdminError, CreateMatrixAdminResponse, ServerConfig, ServerStatus, SessionToken,
    WebUIAdminCredentials,
};

use std::fmt;
use std::pin::Pin;
use std::time::SystemTime;

use clap::Parser;
pub use executor::*;
use futures_util::io::{AsyncWriteExt, BufWriter};
use futures_util::lock::Mutex;
use futures_util::{Future, FutureExt, TryFutureExt};
use regex::Regex;
use tokio::sync::mpsc;

use self::appservice::AppserviceCommand;
use self::federation::FederationCommand;
use self::media::MediaCommand;
use self::room::RoomCommand;
use self::server::ServerCommand;
use self::user::UserCommand;
use crate::AppResult;
use crate::core::ServerName;
use crate::core::events::room::message::RoomMessageEventContent;
use crate::core::identifiers::*;
pub(crate) use crate::macros::admin_command_dispatch;

pub(crate) const PAGE_SIZE: usize = 100;

crate::macros::rustc_flags_capture! {}

/// Inputs to a command are a multi-line string and optional reply_id.
#[derive(Clone, Debug, Default)]
pub struct CommandInput {
    pub command: String,
    pub reply_id: Option<OwnedEventId>,
}
/// Prototype of the tab-completer. The input is buffered text when tab
/// asserted; the output will fully replace the input buffer.
pub type Completer = fn(&str) -> String;

/// Prototype of the command processor. This is a callback supplied by the
/// reloadable admin module.
pub type Processor = fn(CommandInput) -> ProcessorFuture;

/// Return type of the processor
pub type ProcessorFuture = Pin<Box<dyn Future<Output = ProcessorResult> + Send>>;

/// Result wrapping of a command's handling. Both variants are complete message
/// events which have digested any prior errors. The wrapping preserves whether
/// the command failed without interpreting the text. Ok(None) outputs are
/// dropped to produce no response.
pub type ProcessorResult = Result<Option<CommandOutput>, CommandOutput>;

/// Alias for the output structure.
pub type CommandOutput = RoomMessageEventContent;

#[derive(Debug, Parser)]
#[command(name = "palpo", version = crate::info::version())]
pub(crate) enum AdminCommand {
    #[command(subcommand)]
    /// - Commands for managing appservices
    Appservice(AppserviceCommand),

    #[command(subcommand)]
    /// - Commands for managing local users
    User(UserCommand),

    #[command(subcommand)]
    /// - Commands for managing rooms
    Room(RoomCommand),

    #[command(subcommand)]
    /// - Commands for managing federation
    Federation(FederationCommand),

    #[command(subcommand)]
    /// - Commands for managing the server
    Server(ServerCommand),

    #[command(subcommand)]
    /// - Commands for managing media
    Media(MediaCommand),
    // #[command(subcommand)]
    // /// - Commands for debugging things
    // Debug(DebugCommand),
}

#[derive(Debug)]
pub enum AdminRoomEvent {
    ProcessMessage(String),
    SendMessage(RoomMessageEventContent),
}

pub(crate) struct Context<'a> {
    pub(crate) body: &'a [&'a str],
    pub(crate) timer: SystemTime,
    pub(crate) reply_id: Option<&'a EventId>,
    pub(crate) output: Mutex<BufWriter<Vec<u8>>>,
}

impl Context<'_> {
    pub(crate) fn write_fmt(
        &self,
        arguments: fmt::Arguments<'_>,
    ) -> impl Future<Output = AppResult<()>> + Send + '_ + use<'_> {
        let buf = format!("{arguments}");
        self.output.lock().then(async move |mut output| {
            output.write_all(buf.as_bytes()).map_err(Into::into).await
        })
    }

    pub(crate) fn write_str<'a>(
        &'a self,
        s: &'a str,
    ) -> impl Future<Output = AppResult<()>> + Send + 'a {
        self.output
            .lock()
            .then(async move |mut output| output.write_all(s.as_bytes()).map_err(Into::into).await)
    }
}

pub(crate) struct RoomInfo {
    pub(crate) id: OwnedRoomId,
    pub(crate) joined_members: u64,
    pub(crate) name: String,
}

pub(crate) fn get_room_info(room_id: &RoomId) -> RoomInfo {
    RoomInfo {
        id: room_id.to_owned(),
        joined_members: crate::room::joined_member_count(room_id).unwrap_or(0),
        name: crate::room::get_name(room_id).unwrap_or_else(|_| room_id.to_string()),
    }
}

#[tracing::instrument(skip_all, name = "command")]
pub(super) async fn process(command: AdminCommand, context: &Context<'_>) -> AppResult<()> {
    use AdminCommand::*;

    match command {
        Appservice(command) => appservice::process(command, context).await,
        Media(command) => media::process(command, context).await,
        User(command) => user::process(command, context).await,
        Room(command) => room::process(command, context).await,
        Federation(command) => federation::process(command, context).await,
        Server(command) => server::process(command, context).await,
        // Debug(command) => debug::process(command, context).await,
    }
}

/// Maximum number of commands which can be queued for dispatch.
const COMMAND_QUEUE_LIMIT: usize = 512;

pub async fn start() -> AppResult<()> {
    executor::init().await;

    let exec = executor();
    let mut signals = exec.signal.subscribe();
    let (sender, mut receiver) = mpsc::channel(COMMAND_QUEUE_LIMIT);
    _ = exec
        .channel
        .write()
        .expect("locked for writing")
        .insert(sender);

    tokio::task::yield_now().await;
    exec.console.start().await;

    loop {
        tokio::select! {
            command = receiver.recv() => match command {
                Some(command) => exec.handle_command(command).await,
                None => break,
            },
            sig = signals.recv() => match sig {
                Ok(sig) => exec.handle_signal(sig).await,
                Err(_) => continue,
            },
        }
    }

    exec.interrupt().await;

    Ok(())
}

// Utility to turn clap's `--help` text to HTML.
fn usage_to_html(text: &str, server_name: &ServerName) -> String {
    // Replace `@palpo:servername:-subcmdname` with `@palpo:servername: subcmdname`
    let text = text.replace(
        &format!("@palpo:{server_name}:-"),
        &format!("@palpo:{server_name}: "),
    );

    // For the palpo admin room, subcommands become main commands
    let text = text.replace("SUBCOMMAND", "COMMAND");
    let text = text.replace("subcommand", "command");

    // Escape option names (e.g. `<element-id>`) since they look like HTML tags
    let text = text.replace('<', "&lt;").replace('>', "&gt;");

    // Italicize the first line (command name and version text)
    let re = Regex::new("^(.*?)\n").expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "<em>$1</em>\n");

    // Unmerge wrapped lines
    let text = text.replace("\n            ", "  ");

    // Wrap option names in backticks. The lines look like:
    //     -V, --version  Prints version information
    // And are converted to:
    // <code>-V, --version</code>: Prints version information
    // (?m) enables multi-line mode for ^ and $
    let re = Regex::new("(?m)^    (([a-zA-Z_&;-]+(, )?)+)  +(.*)$")
        .expect("Regex compilation should not fail");
    let text = re.replace_all(&text, "<code>$1</code>: $4");

    // Look for a `[commandbody]` tag. If it exists, use all lines below it that
    // start with a `#` in the USAGE section.
    let mut text_lines: Vec<&str> = text.lines().collect();
    let mut command_body = String::new();

    if let Some(line_index) = text_lines.iter().position(|line| *line == "[commandbody]") {
        text_lines.remove(line_index);

        while text_lines
            .get(line_index)
            .map(|line| line.starts_with('#'))
            .unwrap_or(false)
        {
            command_body += if text_lines[line_index].starts_with("# ") {
                &text_lines[line_index][2..]
            } else {
                &text_lines[line_index][1..]
            };
            command_body += "[nobr]\n";
            text_lines.remove(line_index);
        }
    }

    let text = text_lines.join("\n");

    // Improve the usage section
    let text = if command_body.is_empty() {
        // Wrap the usage line in code tags
        let re =
            Regex::new("(?m)^USAGE:\n    (@palpo:.*)$").expect("Regex compilation should not fail");
        re.replace_all(&text, "USAGE:\n<code>$1</code>").to_string()
    } else {
        // Wrap the usage line in a code block, and add a yaml block example
        // This makes the usage of e.g. `register-appservice` more accurate
        let re =
            Regex::new("(?m)^USAGE:\n    (.*?)\n\n").expect("Regex compilation should not fail");
        re.replace_all(&text, "USAGE:\n<pre>$1[nobr]\n[commandbodyblock]</pre>")
            .replace("[commandbodyblock]", &command_body)
    };

    // Add HTML line-breaks

    text.replace("\n\n\n", "\n\n")
        .replace('\n', "<br>\n")
        .replace("[nobr]<br>", "")
}
