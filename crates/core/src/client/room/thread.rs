//! `GET /_matrix/client/*/rooms/{room_id}/threads`
//!
//! Retrieve a list of threads in a room, with optional filters.
//! `/v1/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv1roomsroomidthreads

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::AnyTimelineEvent;
use crate::serde::{RawJson, StringEnum};
use crate::{OwnedRoomId, PrivOwnedStr};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         unstable =>
// "/_matrix/client/unstable/org.matrix.msc3856/rooms/:room_id/threads",
//         1.4 => "/_matrix/client/v1/rooms/:room_id/threads",
//     }
// };

// /// Request type for the `get_thread_roots` endpoint.
#[derive(ToParameters, Deserialize, Debug)]
pub struct ThreadsReqArgs {
    /// The room ID where the thread roots are located.
    #[salvo(parameter(parameter_in = Path))]
    pub room_id: OwnedRoomId,

    /// The pagination token to start returning results from.
    ///
    /// If `None`, results start at the most recent topological event visible to
    /// the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub from: Option<String>,

    /// Which thread roots are of interest to the caller.
    #[serde(default, skip_serializing_if = "crate::serde::is_default")]
    #[salvo(parameter(parameter_in = Query))]
    pub include: IncludeThreads,

    /// The maximum number of results to return in a single `chunk`.
    ///
    /// Servers should apply a default value, and impose a maximum value to
    /// avoid resource exhaustion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[salvo(parameter(parameter_in = Query))]
    pub limit: Option<usize>,
}

/// Response type for the `get_thread_roots` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ThreadsResBody {
    /// The thread roots, ordered by the `latest_event` in each event's
    /// aggregation bundle.
    ///
    /// All events returned include bundled aggregations.
    pub chunk: Vec<RawJson<AnyTimelineEvent>>,

    /// An opaque string to provide to `from` to keep paginating the responses.
    ///
    /// If this is `None`, there are no more results to fetch and the client
    /// should stop paginating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_batch: Option<String>,
}
impl ThreadsResBody {
    /// Creates a new `Response` with the given chunk.
    pub fn new(chunk: Vec<RawJson<AnyTimelineEvent>>) -> Self {
        Self {
            chunk,
            next_batch: None,
        }
    }
}

/// Which threads to include in the response.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, Default, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum IncludeThreads {
    /// `all`
    ///
    /// Include all thread roots found in the room.
    ///
    /// This is the default.
    #[default]
    All,

    /// `participated`
    ///
    /// Only include thread roots for threads where
    /// [`current_user_participated`] is `true`.
    ///
    /// [`current_user_participated`]: https://spec.matrix.org/latest/client-server-api/#server-side-aggregation-of-mthread-relationships
    Participated,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}
