//! (De)serializable types for the [Matrix Server-Server API][federation-api].
//! These types are used by server code.
//!
//! [federation-api]: https://spec.matrix.org/latest/server-server-api/

#![cfg_attr(docsrs, feature(doc_cfg))]
mod serde;

pub mod authenticated_media;
pub mod authentication;
pub mod authorization;
pub mod backfill;
pub mod device;
pub mod directory;
pub mod discovery;
pub mod event;
pub mod key;
pub mod knock;
pub mod media;
pub mod membership;
pub mod openid;
pub mod policy;
pub mod query;
pub mod room;
pub mod space;
pub mod third_party;
pub mod transaction;
