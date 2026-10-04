//! ADR 0011: shared data and policy for Palpo/Hagency Rust workflow integration.
//!
//! This crate does not authenticate a caller, persist a decision, reserve tokens
//! or provision an agent. The HTTP/machine adapters must authenticate first and
//! load current authority from trusted storage. Deserializing an engagement is
//! not evidence that the resource owner accepted its delegation.
//!
//! Authorize inside the same transaction that writes the decision/outbox. The
//! Hagency executor repeats authorization using current state, then atomically
//! reserves capacity and stores the command receipt. No second human verdict is
//! required. See the crate README for the remaining integration boundaries.

pub mod budget;
pub mod ids;
pub mod policy;

pub use ids::*;
pub use policy::*;

pub const CONTRACT_VERSION: u16 = 1;
pub const COORDINATOR_APPROVAL_CAPABILITY: &str = "hagency.coordinator_approval.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid identifier")]
    InvalidIdentifier,
    #[error("invalid token count or revision")]
    InvalidNumber,
    #[error("unsupported contract version or capability")]
    Unsupported,
    #[error("actor is not authorized for this decision")]
    Forbidden,
    #[error("requester self-approval is not authorized")]
    SelfApproval,
    #[error("request or command binding differs from current state")]
    BindingMismatch,
    #[error("engagement is not verified or its delegation is inactive")]
    EngagementUnavailable,
    #[error("command or delegation has expired or is not yet valid")]
    Expired,
    #[error("project is not ready")]
    ProjectUnavailable,
    #[error("resource is not granted to this project")]
    ResourceNotGranted,
    #[error("budget is unallocated")]
    Unallocated,
    #[error("capacity is insufficient")]
    InsufficientCapacity,
    #[error("accounting arithmetic overflow")]
    Overflow,
}
