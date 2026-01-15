pub mod capture;
pub mod color;
pub mod console;
pub mod fmt;
pub mod fmt_span;
mod reload;
mod suppress;

use std::sync::{Arc, OnceLock};

pub use capture::Capture;
pub use console::{ConsoleFormat, ConsoleWriter, is_systemd_mode};
pub use reload::{LogLevelReloadHandles, ReloadHandle};
pub use suppress::Suppress;
pub use tracing::Level;
pub use tracing_core::{Event, Metadata};
pub use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry};

use crate::AppResult;

pub static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Logging subsystem. This is a singleton member of super::Server which holds
/// all logging and tracing related state rather than shoving it all in
/// super::Server directly.
pub struct Logger {
    /// General log level reload handles.
    pub reload: LogLevelReloadHandles,

    /// Tracing capture state for ephemeral/oneshot uses.
    pub capture: std::sync::Arc<capture::State>,
}

impl std::fmt::Debug for Logger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logger").finish_non_exhaustive()
    }
}

pub fn init() -> AppResult<()> {
    let conf = &crate::config::get().logger;

    let reload_handles = LogLevelReloadHandles::default();

    let console_span_events =
        fmt_span::from_str(&conf.span_events).expect("failed to parse span events");

    let console_filter = EnvFilter::builder()
        .with_regex(conf.filter_regex)
        .parse(&conf.level)
        .expect("failed to parse log level");

    let console_layer = tracing_subscriber::fmt::Layer::new()
        .with_ansi(conf.ansi_colors)
        .with_thread_ids(conf.thread_ids)
        .with_span_events(console_span_events)
        .event_format(ConsoleFormat::new(conf))
        .fmt_fields(ConsoleFormat::new(conf))
        .with_writer(ConsoleWriter::new(conf));

    let (console_reload_filter, _console_reload_handle) =
        tracing_subscriber::reload::Layer::new(console_filter);

    // TODO: fix https://github.com/tokio-rs/tracing/pull/2956
    // reload_handles.add("console", Box::new(console_reload_handle));

    let cap_state = Arc::new(capture::State::new());
    let cap_layer = capture::Layer::new(&cap_state);

    let subscriber = Registry::default()
        .with(console_layer.with_filter(console_reload_filter))
        .with(cap_layer);
    tracing::subscriber::set_global_default(subscriber)
        .expect("the global default tracing subscriber failed to be initialized");

    let logger = Logger {
        reload: reload_handles,
        capture: cap_state,
    };
    LOGGER.set(logger).expect("logger should be set only once");

    Ok(())
}

pub fn get() -> &'static Logger {
    LOGGER.get().expect("Logger not initialized")
}

// // Wraps for logging macros.

// #[macro_export]
// #[collapse_debuginfo(yes)]
// macro_rules! event {
// 	( $level:expr_2021, $($x:tt)+ ) => { ::tracing::event!( $level, $($x)+ ) }
// }

// #[macro_export]
// macro_rules! error {
//     ( $($x:tt)+ ) => { ::tracing::error!( $($x)+ ) }
// }

// #[macro_export]
// macro_rules! warn {
//     ( $($x:tt)+ ) => { ::tracing::warn!( $($x)+ ) }
// }

// #[macro_export]
// macro_rules! info {
//     ( $($x:tt)+ ) => { ::tracing::info!( $($x)+ ) }
// }

// #[macro_export]
// macro_rules! debug {
//     ( $($x:tt)+ ) => { ::tracing::debug!( $($x)+ ) }
// }

// #[macro_export]
// macro_rules! trace {
//     ( $($x:tt)+ ) => { ::tracing::trace!( $($x)+ ) }
// }
