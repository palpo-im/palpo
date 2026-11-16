//! Wire types for `POST
//! /_matrix/client/unstable/org.continuwuity.msc4560.stripped_state/{room_id_or_alias}`.
//!
//! MSC4560 permits optional access-token authentication and requires rate
//! limiting. These types alone do not install a server route.

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::events::AnyStrippedStateEvent;
use crate::serde::RawJson;
use crate::stripped_state::MAX_VIAS;
pub use crate::stripped_state::StrippedStateMsc4560ReqBody;
use crate::{MatrixError, MatrixResult, OwnedRoomOrAliasId, OwnedServerName};

/// Path and query parameters for a stripped-state client request.
#[derive(ToParameters, Debug, Deserialize)]
pub struct StrippedStateMsc4560ReqArgs {
    #[salvo(parameter(parameter_in = Path))]
    pub room_id_or_alias: OwnedRoomOrAliasId,

    /// Candidate servers to try when the local server is not resident.
    /// Count entries before deduplicating, including repeated server names.
    #[salvo(parameter(parameter_in = Query))]
    #[serde(default)]
    pub via: Vec<OwnedServerName>,
}

impl StrippedStateMsc4560ReqArgs {
    /// Reject more than 32 entries with the proposal's `M_TOO_LARGE` error.
    pub fn validate(&self) -> MatrixResult<()> {
        if self.via.len() > MAX_VIAS {
            let mut error = MatrixError::too_large("MSC4560 permits at most 32 via entries");
            // MSC4560 specifies HTTP 400 for this query limit, whereas the
            // general M_TOO_LARGE helper defaults to HTTP 413 for bodies.
            error.status_code = Some(StatusCode::BAD_REQUEST);
            return Err(error);
        }
        Ok(())
    }
}

/// Client response. `via` names the server that actually supplied the data.
#[derive(ToSchema, Debug, Deserialize, Serialize)]
pub struct StrippedStateMsc4560ResBody {
    pub state: Vec<RawJson<AnyStrippedStateEvent>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_joined_members: Option<u64>,

    pub via: OwnedServerName,
}

impl StrippedStateMsc4560ResBody {
    pub fn new(via: OwnedServerName) -> Self {
        Self {
            state: Vec::new(),
            num_joined_members: None,
            via,
        }
    }
}
