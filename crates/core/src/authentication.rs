//! Common types for authentication.

use salvo::prelude::*;

use crate::PrivOwnedStr;
use crate::serde::StringEnum;

/// Access token types.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[non_exhaustive]
pub enum TokenType {
    /// Bearer token type
    Bearer,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}
