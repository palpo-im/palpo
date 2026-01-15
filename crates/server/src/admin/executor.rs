use std::sync::{Arc, OnceLock, RwLock as StdRwLock};

use clap::Parser;
use tokio::sync::{RwLock, broadcast, mpsc};

use crate::admin::{
    AdminCommand, CommandInput, Completer, Console, Processor, ProcessorResult, processor,
};
use crate::core::events::room::message::{Relation, RoomMessageEventContent};
use crate::core::identifiers::*;
use crate::room::timeline;
use crate::{AppError, AppResult, PduBuilder, RoomMutexGuard, config};

pub static EXECUTOR: OnceLock<Executor> = OnceLock::new();
pub fn executor() -> &'static Executor {
    EXECUTOR.get().expect("executor not initialized")
}

pub async fn init() {
    let exec = Executor {
        signal: broadcast::channel::<&'static str>(1).0,
        channel: StdRwLock::new(None),
        handle: RwLock::new(None),
        complete: StdRwLock::new(None),
        console: Console::new(),
    };
    _ = exec
        .complete
        .write()
        .expect("locked for writing")
        .insert(processor::complete);
    _ = exec.handle.write().await.insert(processor::dispatch);
    EXECUTOR.set(exec).expect("executor already initialized");
}

pub struct Executor {
    pub signal: broadcast::Sender<&'static str>,
    pub channel: StdRwLock<Option<mpsc::Sender<CommandInput>>>,
    pub handle: RwLock<Option<Processor>>,
    pub complete: StdRwLock<Option<Completer>>,
    pub console: Arc<Console>,
}
impl std::fmt::Debug for Executor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Executor").finish()
    }
}
impl Executor {
    pub(super) async fn handle_signal(&self, sig: &'static str) {
        if sig == SIGNAL {
            self.signal_execute().await.ok();
        }

        self.console.handle_signal(sig).await;
    }

    pub(super) async fn signal_execute(&self) -> AppResult<()> {
        let conf = config::get();
        // List of commands to execute
        let commands = conf.admin.signal_execute.clone();

        // When true, errors are ignored and execution continues.
        let ignore_errors = conf.admin.execute_errors_ignore;

        for (i, command) in commands.iter().enumerate() {
            if let Err(e) = self.execute_command(i, command.clone()).await
                && !ignore_errors
            {
                return Err(e);
            }

            tokio::task::yield_now().await;
        }

        Ok(())
    }

    /// Posts a command to the command processor queue and returns. Processing
    /// will take place on the service worker's task asynchronously. Errors if
    /// the queue is full.
    pub async fn command(&self, command: String, reply_id: Option<OwnedEventId>) -> AppResult<()> {
        let Some(sender) = self.channel.read().expect("locked for reading").clone() else {
            return Err(AppError::public("admin command queue unavailable"));
        };

        sender
            .send(CommandInput { command, reply_id })
            .await
            .map_err(|e| AppError::Public(format!("failed to enqueue admin command: {e:?}")))
    }
    async fn execute_command(&self, i: usize, command: String) -> AppResult<()> {
        debug!("Execute command #{i}: executing {command:?}");

        match self.command_in_place(command, None).await {
            Ok(Some(output)) => Self::execute_command_output(i, &output),
            Err(output) => Self::execute_command_error(i, &output),
            Ok(None) => {
                info!("Execute command #{i} completed (no output).");
                Ok(())
            }
        }
    }

    /// Dispatches a command to the processor on the current task and waits for
    /// completion.
    pub async fn command_in_place(
        &self,
        command: String,
        reply_id: Option<OwnedEventId>,
    ) -> ProcessorResult {
        self.process_command(CommandInput { command, reply_id })
            .await
    }

    fn execute_command_output(i: usize, content: &RoomMessageEventContent) -> AppResult<()> {
        info!("Execute command #{i} completed:");
        super::console::print(content.body());
        Ok(())
    }
    fn execute_command_error(i: usize, content: &RoomMessageEventContent) -> AppResult<()> {
        super::console::print_err(content.body());
        error!("Execute command #{i} failed.");
        Err(AppError::Public(format!(
            "Execute command #{i} failed: {}",
            content.body()
        )))
    }

    pub(super) async fn handle_command(&self, command: CommandInput) {
        match self.process_command(command).await {
            Ok(None) => debug!("Command successful with no response"),
            Ok(Some(output)) | Err(output) => self.handle_response(output).await.unwrap(),
        }
    }

    /// Invokes the tab-completer to complete the command. When unavailable,
    /// None is returned.
    pub fn complete_command(&self, command: &str) -> Option<String> {
        self.complete
            .read()
            .expect("locked for reading")
            .map(|complete| complete(command))
    }
    async fn process_command(&self, command: CommandInput) -> ProcessorResult {
        let handle = &self
            .handle
            .read()
            .await
            .expect("Admin module is not loaded");
        handle(command).await
    }

    async fn handle_response(&self, content: RoomMessageEventContent) -> AppResult<()> {
        let Some(Relation::Reply { in_reply_to }) = content.relates_to.as_ref() else {
            return Ok(());
        };

        let Ok(pdu) = timeline::get_pdu(&in_reply_to.event_id) else {
            error!(
                event_id = ?in_reply_to.event_id,
                "Missing admin command in_reply_to event"
            );
            return Ok(());
        };

        let response_sender = if crate::room::is_admin_room(&pdu.room_id)? {
            config::server_user_id()
        } else {
            &pdu.sender
        };

        respond_to_room(content, &pdu.room_id, response_sender).await
    }

    pub(super) async fn interrupt(&self) {
        // TODO: not unwind safe
        self.console.interrupt();
        _ = self.channel.write().expect("locked for writing").take();
        self.console.close().await;
    }
}

/// Sends markdown notice to the admin room as the admin user.
pub async fn send_notice(body: &str) -> AppResult<()> {
    send_message(RoomMessageEventContent::notice_markdown(body)).await
}

/// Sends markdown message (not an m.notice for notification reasons) to the
/// admin room as the admin user.
pub async fn send_text(body: &str) -> AppResult<()> {
    send_message(RoomMessageEventContent::text_markdown(body)).await
}

/// Sends a message to the admin room as the admin user (see send_text() for
/// convenience).
pub async fn send_message(message_content: RoomMessageEventContent) -> AppResult<()> {
    let user_id = &config::server_user_id();
    let room_id = crate::room::get_admin_room()?;
    respond_to_room(message_content, &room_id, user_id).await
}

pub(super) const SIGNAL: &str = "SIGUSR2";

// Parse chat messages from the admin room into an AdminCommand object
fn parse_admin_command(command_line: &str) -> std::result::Result<AdminCommand, String> {
    // Note: argv[0] is `@palpo:servername:`, which is treated as the main command
    let mut argv: Vec<_> = command_line.split_whitespace().collect();

    // Replace `help command` with `command --help`
    // Clap has a help subcommand, but it omits the long help description.
    if argv.len() > 1 && argv[1] == "help" {
        argv.remove(1);
        argv.push("--help");
    }

    // Backwards compatibility with `register_appservice`-style commands
    let command_with_dashes;
    if argv.len() > 1 && argv[1].contains('_') {
        command_with_dashes = argv[1].replace('_', "-");
        argv[1] = &command_with_dashes;
    }

    AdminCommand::try_parse_from(argv).map_err(|error| error.to_string())
}

async fn respond_to_room(
    content: RoomMessageEventContent,
    room_id: &RoomId,
    user_id: &UserId,
) -> AppResult<()> {
    assert!(crate::room::is_admin_room(room_id)?, "sender is not admin");

    let state_lock = crate::room::lock_state(room_id).await;

    if let Err(e) = timeline::build_and_append_pdu(
        PduBuilder::timeline(&content),
        user_id,
        room_id,
        &crate::room::get_version(room_id)?,
        &state_lock,
    )
    .await
    {
        handle_response_error(e, room_id, user_id, &state_lock).await?;
    }

    Ok(())
}

async fn handle_response_error(
    e: AppError,
    room_id: &RoomId,
    user_id: &UserId,
    state_lock: &RoomMutexGuard,
) -> AppResult<()> {
    error!("Failed to build and append admin room response PDU: \"{e}\"");
    let content = RoomMessageEventContent::text_plain(format!(
        "Failed to build and append admin room PDU: \"{e}\"\n\nThe original admin command \
			 may have finished successfully, but we could not return the output."
    ));

    timeline::build_and_append_pdu(
        PduBuilder::timeline(&content),
        user_id,
        room_id,
        &crate::room::get_version(room_id)?,
        state_lock,
    )
    .await?;

    Ok(())
}
