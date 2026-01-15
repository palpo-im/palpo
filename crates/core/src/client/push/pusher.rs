//! `GET /_matrix/client/*/pushers`
//!
//! Gets all currently active pushers for the authenticated user.

use js_option::JsOption;
use salvo::prelude::*;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, de};

use crate::push::{Pusher, PusherIds};
use crate::serde::{RawJsonValue, from_raw_json_value};

// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3pushers
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/pushers",
//         1.1 => "/_matrix/client/v3/pushers",
//     }
// };
/// Response type for the `get_pushers` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct PushersResBody {
    /// An array containing the current pushers for the user.
    pub pushers: Vec<Pusher>,
}

impl PushersResBody {
    /// Creates a new `Response` with the given pushers.
    pub fn new(pushers: Vec<Pusher>) -> Self {
        Self { pushers }
    }
}

// `POST /_matrix/client/*/pushers/set`
//
// This endpoint allows the creation, modification and deletion of pushers for
// this user ID. `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3pushersset

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: true,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/pushers/set",
//         1.1 => "/_matrix/client/v3/pushers/set",
//     }
// };

/// Request type for the `set_pusher` endpoint.
#[derive(ToSchema, Deserialize, Debug)]
pub struct SetPusherReqBody(pub PusherAction);

// impl Request {
//     /// Creates a new `Request` for the given action.
//     pub fn new(action: PusherAction) -> Self {
//         Self { action }
//     }

//     /// Creates a new `Request` to create or update the given pusher.
//     pub fn post(pusher: Pusher) -> Self {
//         Self::new(PusherAction::Post(PusherPostData { pusher, append: false
// }))     }

//     /// Creates a new `Request` to delete the pusher identified by the given
// IDs.     pub fn delete(ids: PusherIds) -> Self {
//         Self::new(PusherAction::Delete(ids))
//     }
// }

/// The action to take for the pusher.
#[derive(ToSchema, Clone, Debug)]
pub enum PusherAction {
    /// Create or update the given pusher.
    Post(PusherPostData),

    /// Delete the pusher identified by the given IDs.
    Delete(PusherIds),
}
impl Serialize for PusherAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PusherAction::Post(pusher) => pusher.serialize(serializer),
            PusherAction::Delete(ids) => {
                let mut st = serializer.serialize_struct("PusherAction", 3)?;
                st.serialize_field("pushkey", &ids.pushkey)?;
                st.serialize_field("app_id", &ids.app_id)?;
                st.serialize_field("kind", &None::<&str>)?;
                st.end()
            }
        }
    }
}
#[derive(Debug, Deserialize)]
struct PusherActionDeHelper {
    kind: JsOption<String>,
}

impl<'de> Deserialize<'de> for PusherAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;
        let PusherActionDeHelper { kind } = from_raw_json_value(&json)?;

        match kind {
            JsOption::Some(_) => Ok(Self::Post(from_raw_json_value(&json)?)),
            JsOption::Null => Ok(Self::Delete(from_raw_json_value(&json)?)),
            // This is unreachable because we don't use `#[serde(default)]` on the field.
            JsOption::Undefined => Err(de::Error::missing_field("kind")),
        }
    }
}

/// Data necessary to create or update a pusher.
#[derive(ToSchema, Serialize, Clone, Debug)]
pub struct PusherPostData {
    /// The pusher to configure.
    #[serde(flatten)]
    pub pusher: Pusher,

    /// Controls if another pusher with the same pushkey and app id should be
    /// created, if there are already others for other users.
    ///
    /// Defaults to `false`. See the spec for more details.
    #[serde(
        skip_serializing_if = "crate::serde::is_default",
        default = "default_false"
    )]
    pub append: bool,
}

#[derive(Debug, Deserialize)]
struct PusherPostDataDeHelper {
    #[serde(default)]
    append: bool,
}
impl<'de> Deserialize<'de> for PusherPostData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let json = Box::<RawJsonValue>::deserialize(deserializer)?;

        let PusherPostDataDeHelper { append } = from_raw_json_value(&json)?;
        let pusher = from_raw_json_value(&json)?;

        Ok(Self { pusher, append })
    }
}
