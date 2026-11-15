//! Experimental MSC4536 batch profile queries.

use std::collections::BTreeMap;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::OwnedUserId;
use crate::profile::ProfileFieldName;
use crate::serde::JsonObject;

/// Request for `POST /_matrix/client/unstable/org.matrix.msc4536/profile/query`.
#[derive(Debug, ToSchema, Serialize, Deserialize)]
pub struct QueryProfilesReqBody {
    /// Between 1 and 100 user IDs, before deduplication.
    pub users: Vec<OwnedUserId>,
    /// When present, 1 to 30 nonempty field names of at most 255 UTF-8 bytes.
    /// Omission requests all fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<ProfileFieldName>>,
}

/// Profiles keyed directly by user ID, without a response envelope.
pub type QueryProfilesResBody = BTreeMap<OwnedUserId, JsonObject>;
