//! Endpoint for pinging the application service.

//! `PUT /_matrix/app/*/ping`
//!
//! Endpoint to ping the application service.
//! `/v1/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/application-service-api/#post_matrixappv1ping
use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::OwnedTransactionId;
use crate::sending::{SendRequest, SendResult};

// const METADATA: Metadata = metadata! {
//     method: POST,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         unstable => "/_matrix/app/unstable/fi.mau.msc2659/ping",
//         1.7 => "/_matrix/app/v1/ping",
//     }
// };

pub fn send_ping_request(dest: &str, body: SendPingReqBody) -> SendResult<SendRequest> {
    let url = Url::parse(dest)?;
    crate::sending::post(url).stuff(body)
}

/// Request type for the `send_ping` endpoint.
#[derive(ToSchema, Deserialize, Serialize, Debug)]
pub struct SendPingReqBody {
    /// A transaction ID for the ping, copied directly from the `POST
    /// /_matrix/client/v1/appservice/{appserviceId}/ping` call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<OwnedTransactionId>,
}
crate::json_body_modifier!(SendPingReqBody);

/// Response type for the `request_ping` endpoint.
#[derive(ToSchema, Serialize, Default, Debug)]
pub struct SendPingResBody {
    #[serde(with = "crate::serde::duration::ms", rename = "duration_ms")]
    pub duration: Duration,
}

impl SendPingResBody {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }
}
