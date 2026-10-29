use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use ulid::Ulid;

use crate::core::Seqnum;
use crate::core::client::dehydrated_device::{
    DehydratedDeviceEventsReqArgs, DehydratedDeviceEventsReqBody, DehydratedDeviceEventsResBody,
};
use crate::core::device::DirectDeviceContent;
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::to_device::{
    DeviceIdOrAllDevices, SendEventToDeviceReqArgs, SendEventToDeviceReqBody,
};
use crate::{
    AppResult, AuthArgs, DepotExt, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, data,
    empty_ok, json_ok,
};

pub fn authed_router() -> Router {
    Router::with_path("sendToDevice/{event_type}/{txn_id}").put(send_to_device)
}

/// #PUT /_matrix/client/r0/sendToDevice/{event_type}/{txn_id}
/// Send a to-device event to a set of client devices.
#[endpoint]
async fn send_to_device(
    _aa: AuthArgs,
    args: SendEventToDeviceReqArgs,
    body: JsonBody<SendEventToDeviceReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;
    let mut inboxes = Vec::new();
    let mut outgoing = Vec::new();
    let mut destinations = std::collections::BTreeSet::new();
    for (target_user_id, map) in &body.messages {
        for (target_device_id_maybe, event) in map {
            let content: serde_json::Value = event
                .deserialize_as()
                .map_err(|_| MatrixError::invalid_param("Event is invalid"))?;
            if target_user_id.server_name().is_remote() {
                let server = target_user_id.server_name();
                if !crate::config::get().federation.is_server_allowed(server) {
                    continue;
                }
                let mut map = BTreeMap::new();
                map.insert(target_device_id_maybe.clone(), event.clone());
                let mut messages = BTreeMap::new();
                messages.insert(target_user_id.clone(), map);

                let message_id = Ulid::generate();
                let edu = Edu::DirectToDevice(DirectDeviceContent {
                    sender: authed.user_id().to_owned(),
                    ev_type: args.event_type.clone(),
                    message_id: message_id.to_string().into(),
                    messages,
                });
                outgoing.push(data::sending::NewDbOutgoingRequest {
                    kind: "normal".to_owned(),
                    appservice_id: None,
                    user_id: None,
                    pushkey: None,
                    server_id: Some(server.to_owned()),
                    pdu_id: None,
                    edu_json: Some(serde_json::to_vec(&edu)?),
                });
                destinations.insert(server.to_owned());

                continue;
            }

            let targets = match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(device) => vec![device.clone()],
                DeviceIdOrAllDevices::AllDevices => {
                    data::user::all_to_device_target_ids(target_user_id).await?
                }
            };
            for device in targets {
                inboxes.push(data::user::device::NewDbDeviceInbox {
                    user_id: target_user_id.clone(), device_id: device,
                    json_data: serde_json::json!({"type":args.event_type,"sender":authed.user_id(),"content":content}),
                    created_at: crate::core::UnixMillis::now().get() as i64,
                });
            }
        }
    }

    let receipt = data::room::NewDbEventIdempotent {
        txn_id: args.txn_id.clone(),
        user_id: authed.user_id().to_owned(),
        device_id: authed.device_id().map(ToOwned::to_owned),
        room_id: None,
        event_id: None,
        created_at: crate::core::UnixMillis::now(),
        endpoint: format!("sendToDevice/{}", args.event_type),
    };
    if data::room::transaction_id::commit_to_device_transaction(&receipt, &inboxes, &outgoing)
        .await?
    {
        for server in destinations {
            crate::sending::notify(crate::sending::OutgoingKind::Normal(server))?;
        }
    }

    empty_ok()
}

/// How many to-device messages one batch carries when the client does not say.
const DEFAULT_EVENT_LIMIT: usize = 100;

/// An upper bound on `limit`, so one request cannot be made to load an unbounded inbox.
const MAX_EVENT_LIMIT: usize = 1000;

/// #GET /_matrix/client/unstable/org.matrix.msc3814.v1/dehydrated_device/{device_id}/events
/// Retrieves the to-device messages sent to a dehydrated device ([MSC3814]).
///
/// Reading does not consume: an interrupted rehydration must be restartable, possibly by a
/// different device, so messages already returned stay available. `next_batch` is present
/// only while more messages remain, which is how the client knows it has everything and
/// can replace the dehydrated device.
///
/// [MSC3814]: https://github.com/matrix-org/matrix-spec-proposals/pull/3814
#[endpoint]
pub(super) async fn for_dehydrated(
    _aa: AuthArgs,
    args: DehydratedDeviceEventsReqArgs,
    depot: &mut Depot,
) -> JsonResult<DehydratedDeviceEventsResBody> {
    let authed = depot.authed_info()?;
    let (body, _) = collect_dehydrated_events(
        authed.user_id(),
        &args.device_id,
        args.from.as_deref(),
        args.limit,
    )
    .await?;
    json_ok(body)
}

/// #POST /_matrix/client/unstable/org.matrix.msc3814.v1/dehydrated_device/{device_id}/events
/// Retrieves the to-device messages sent to a dehydrated device ([MSC3814]).
///
/// The `POST` form comes from an earlier draft of the MSC and is kept for clients that
/// still use it. It differs from `GET` in taking the resume token in the body and in
/// always returning `next_batch`; those clients stop when a response comes back empty
/// rather than when the token is absent, so the token must always move past what was
/// just returned.
///
/// [MSC3814]: https://github.com/matrix-org/matrix-spec-proposals/pull/3814
#[endpoint]
pub(super) async fn for_dehydrated_legacy(
    _aa: AuthArgs,
    args: DehydratedDeviceEventsReqArgs,
    body: JsonBody<DehydratedDeviceEventsReqBody>,
    depot: &mut Depot,
) -> JsonResult<DehydratedDeviceEventsResBody> {
    let authed = depot.authed_info()?;
    let requested_from = body.into_inner().next_batch;

    let (mut body, position) = collect_dehydrated_events(
        authed.user_id(),
        &args.device_id,
        requested_from.as_deref(),
        args.limit,
    )
    .await?;

    body.next_batch = Some(legacy_next_batch(
        body.next_batch.take(),
        position,
        requested_from,
    ));

    json_ok(body)
}

async fn collect_dehydrated_events(
    user_id: &UserId,
    device_id: &DeviceId,
    from: Option<&str>,
    limit: Option<usize>,
) -> AppResult<(DehydratedDeviceEventsResBody, Option<Seqnum>)> {
    // Only the user's current dehydrated device may be read this way; any other device ID
    // -- including one of the user's live devices -- is forbidden, per MSC3814.
    let dehydrated_device_id = data::user::get_dehydrated_device(user_id)
        .await?
        .map(|(device_id, _)| device_id);
    if dehydrated_device_id.as_deref() != Some(device_id) {
        return Err(MatrixError::forbidden(
            "The given device ID is not the user's dehydrated device.",
            None,
        )
        .into());
    }

    let since_sn = parse_batch_token(from)?;
    let (events, has_more) = data::user::device::to_device_events_from(
        user_id,
        device_id,
        since_sn,
        resolve_limit(limit),
    )
    .await?;

    // The batch is the last one when nothing follows its final message. Deciding this from
    // the number of events returned would be wrong when the inbox happens to hold exactly
    // `limit` more.
    let position = events.last().map(|(occur_sn, _)| *occur_sn);
    let next_batch = position
        .filter(|_| has_more)
        .map(|last_sn| last_sn.to_string());

    Ok((
        DehydratedDeviceEventsResBody {
            events: events.into_iter().map(|(_, event)| event).collect(),
            next_batch,
        },
        position,
    ))
}

/// The token the deprecated `POST` form returns.
///
/// That form always returns a token, so a client cannot use its absence to detect the last
/// batch and instead stops when a response comes back empty. The token must therefore still
/// advance past whatever was just returned: handing back the request's own token on a final
/// non-empty batch would return the same batch forever.
fn legacy_next_batch(
    computed: Option<String>,
    position: Option<Seqnum>,
    requested: Option<String>,
) -> String {
    computed
        .or_else(|| position.map(|position| position.to_string()))
        .or(requested)
        .unwrap_or_else(|| "0".to_owned())
}

/// Parses a `next_batch` / `from` token into the stream position it names.
fn parse_batch_token(from: Option<&str>) -> Result<Option<Seqnum>, MatrixError> {
    from.map(|from| {
        from.parse::<Seqnum>()
            .map_err(|_| MatrixError::invalid_param("Invalid next_batch token."))
    })
    .transpose()
}

/// The batch size to use, bounded so one request cannot pull an entire inbox into memory.
fn resolve_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_EVENT_LIMIT)
        .clamp(1, MAX_EVENT_LIMIT)
}

#[cfg(test)]
mod transaction_tests {
    use diesel::prelude::*;
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use futures_util::stream::{FuturesUnordered, StreamExt};
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;
    use crate::data::connect;
    use crate::data::schema::*;

    async fn send(
        service: &Service,
        event_type: &str,
        txn: &str,
        body: &Value,
    ) -> (Option<StatusCode>, Value) {
        let mut response =
            TestClient::put(format!("http://localhost/sendToDevice/{event_type}/{txn}"))
                .add_header("authorization", "Bearer scoped-sender", true)
                .json(body)
                .send(service)
                .await;
        (response.status_code, response.take_json().await.unwrap())
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_client_transactions_are_scoped_atomic_and_concurrent() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"transactions.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let user =
            UserId::parse(format!("@scopedsender:{}", crate::config::server_name())).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user.localpart().into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: crate::core::UnixMillis::now(),
        })
        .await
        .unwrap();
        let device: OwnedDeviceId = "SCOPED_SENDER".into();
        data::user::device::create_device(&user, &device, "scoped-sender", None, None)
            .await
            .unwrap();
        let room = crate::core::room_id!("!scoped:example.test");
        data::room::transaction_id::add_scoped_txn_id(
            "same".into(),
            &user,
            Some(&device),
            Some(room),
            Some(crate::core::event_id!("$scoped")),
            "room/send/m.room.message",
        )
        .await
        .unwrap();
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(authed_router()),
        );
        let body = json!({"messages":{user.as_str():{device.as_str():{"fixture":"content"}}}});
        let mut sends = FuturesUnordered::new();
        for _ in 0..12 {
            sends.push(send(&service, "m.first", "same", &body));
        }
        while let Some((status, body)) = sends.next().await {
            assert_eq!(status, Some(StatusCode::OK), "{body}");
        }
        let events = data::user::device::get_to_device_events(&user, &device, None, None)
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "retries duplicate a to-device delivery");
        let (status, body_response) = send(&service, "m.second", "same", &body).await;
        assert_eq!(status, Some(StatusCode::OK), "{body_response}");
        assert_eq!(
            data::user::device::get_to_device_events(&user, &device, None, None)
                .await
                .unwrap()
                .len(),
            2
        );
        // A failure after receipt reservation must roll the receipt and deliveries
        // back, so the same transaction remains retryable.
        connect().await.unwrap().batch_execute("CREATE FUNCTION q14_fail_delivery() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.json_data->>'type' = 'm.fail' THEN RAISE EXCEPTION 'injected inbox failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER q14_fail_delivery BEFORE INSERT ON device_inboxes FOR EACH ROW EXECUTE FUNCTION q14_fail_delivery();").await.unwrap();
        assert_eq!(
            send(&service, "m.fail", "retry", &body).await.0,
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
        let receipts: i64 = event_idempotents::table
            .filter(event_idempotents::user_id.eq(&user))
            .filter(event_idempotents::endpoint.eq("sendToDevice/m.fail"))
            .count()
            .get_result(&mut connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(receipts, 0);
        connect().await.unwrap().batch_execute("DROP TRIGGER q14_fail_delivery ON device_inboxes; DROP FUNCTION q14_fail_delivery();").await.unwrap();
        let (status, body_response) = send(&service, "m.fail", "retry", &body).await;
        assert_eq!(status, Some(StatusCode::OK), "{body_response}");
        assert_eq!(
            data::user::device::get_to_device_events(&user, &device, None, None)
                .await
                .unwrap()
                .len(),
            3
        );
    }
}

#[cfg(test)]
mod dehydrated_tests {
    use super::{
        DEFAULT_EVENT_LIMIT, MAX_EVENT_LIMIT, legacy_next_batch, parse_batch_token, resolve_limit,
    };

    #[test]
    fn batch_tokens_are_stream_positions() {
        assert_eq!(parse_batch_token(None).unwrap(), None);
        assert_eq!(parse_batch_token(Some("42")).unwrap(), Some(42));

        // A token the server never issued must be rejected rather than silently restarting
        // the client from the beginning of the inbox.
        assert!(parse_batch_token(Some("")).is_err());
        assert!(parse_batch_token(Some("abc")).is_err());
        assert!(parse_batch_token(Some("1.5")).is_err());
    }

    #[test]
    fn the_legacy_token_always_moves_forward() {
        // More to come: use the token the batch itself produced.
        assert_eq!(
            legacy_next_batch(Some("9".to_owned()), Some(9), Some("4".to_owned())),
            "9"
        );
        // Final non-empty batch: the GET form omits the token, but this form must still
        // advance past the messages returned, or the client asks for them forever.
        assert_eq!(legacy_next_batch(None, Some(9), Some("4".to_owned())), "9");
        // Nothing returned: stay where the client already was.
        assert_eq!(legacy_next_batch(None, None, Some("4".to_owned())), "4");
        // Nothing returned and no prior token: the start of the stream.
        assert_eq!(legacy_next_batch(None, None, None), "0");
    }

    #[test]
    fn the_batch_size_is_bounded() {
        assert_eq!(resolve_limit(None), DEFAULT_EVENT_LIMIT);
        assert_eq!(resolve_limit(Some(10)), 10);
        assert_eq!(resolve_limit(Some(usize::MAX)), MAX_EVENT_LIMIT);
        // A zero limit would return an empty batch forever, so it never makes progress.
        assert_eq!(resolve_limit(Some(0)), 1);
    }
}
