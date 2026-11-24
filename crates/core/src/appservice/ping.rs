//! `POST /_matrix/app/v1/ping` ([spec]).
//!
//! [spec]: https://spec.matrix.org/latest/application-service-api/#post_matrixappv1ping
use std::time::Duration;

use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::OwnedTransactionId;
use crate::sending::{SendError, SendRequest, SendResult};

pub fn send_ping_request(dest: &str, body: SendPingReqBody) -> SendResult<SendRequest> {
    let mut url = Url::parse(dest)?;
    url.path_segments_mut()
        .map_err(|()| SendError::other("appservice URL cannot be used as a base URL"))?
        .pop_if_empty()
        .extend(["_matrix", "app", "v1", "ping"]);
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
    #[salvo(schema(value_type = u64))]
    pub duration: Duration,
}

impl SendPingResBody {
    pub fn new(duration: Duration) -> Self {
        Self { duration }
    }
}

#[cfg(test)]
mod tests {
    use salvo::conn::Acceptor;
    use salvo::prelude::*;
    use serde_json::{Value, json};

    use super::*;

    #[handler]
    fn bridge(req: &mut Request) -> Json<Value> {
        Json(json!({"method": req.method().as_str(), "path": req.uri().path()}))
    }

    #[tokio::test]
    async fn ping_request_targets_appservice_endpoint() {
        let listener = TcpListener::new("127.0.0.1:0").bind().await;
        let address = listener.holdings()[0]
            .local_addr
            .clone()
            .into_std()
            .unwrap();
        let peer =
            tokio::spawn(Server::new(listener).serve(Router::with_path("{*path}").goal(bridge)));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();

        for (prefix, expected_path) in [
            ("", "/_matrix/app/v1/ping"),
            ("/", "/_matrix/app/v1/ping"),
            ("/relay", "/relay/_matrix/app/v1/ping"),
            ("/relay/", "/relay/_matrix/app/v1/ping"),
        ] {
            let request = send_ping_request(
                &format!("http://{address}{prefix}"),
                SendPingReqBody {
                    transaction_id: None,
                },
            )
            .unwrap();
            let received = request
                .send_by_client::<Value>(client.clone())
                .await
                .unwrap();
            assert_eq!(received, json!({"method": "POST", "path": expected_path}));
        }
        peer.abort();
    }
}
