use serde::Deserialize;

use crate::core::serde::{default_false, default_true};
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "read_receipt")]
#[derive(Clone, Debug, Deserialize, Default)]
pub struct ReadReceiptConfig {
    /// Allow receiving incoming read receipts from remote servers.
    #[serde(default = "default_true")]
    pub allow_incoming: bool,

    /// Allow sending read receipts to remote servers.
    #[serde(default = "default_true")]
    pub allow_outgoing: bool,
}
