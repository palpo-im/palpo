use std::collections::BTreeMap;
use std::time::Instant;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Deserialize;

use crate::core::UnixMillis;
use crate::core::device::{DeviceListUpdateContent, DirectDeviceContent};
use crate::core::events::receipt::{
    ReceiptContent, ReceiptEvent, ReceiptEventContent, ReceiptType,
};
use crate::core::events::typing::TypingContent;
use crate::core::federation::transaction::{Edu, SendMessageResBody, SigningKeyUpdateContent};
use crate::core::identifiers::*;
use crate::core::presence::PresenceContent;
#[cfg(feature = "unstable-msc4495")]
use crate::core::presence::PresenceUpdate;
use crate::core::serde::RawJsonValue;
use crate::core::to_device::DeviceIdOrAllDevices;
use crate::data::user::NewDbPresence;
use crate::event::{handler, parse_incoming_pdu};
use crate::sending::{EDU_LIMIT, PDU_LIMIT};
use crate::{AppError, AppResult, DepotExt, JsonResult, MatrixError, data, json_ok, room};

pub fn router() -> Router {
    Router::with_path("send/{txn_id}").put(send_message)
}

/// Decode ephemeral events independently so one malformed EDU cannot reject the
/// transaction's persistent events. Keep every raw EDU until the limit is checked.
#[derive(Deserialize, ToSchema)]
struct IncomingTransaction {
    origin: OwnedServerName,
    #[serde(rename = "origin_server_ts")]
    _origin_server_ts: UnixMillis,
    // Match the outgoing request's enabled `unstable-unspecified` compatibility.
    #[serde(default)]
    #[salvo(schema(value_type = Vec<Object>))]
    pdus: Vec<Box<RawJsonValue>>,
    #[serde(default)]
    #[salvo(schema(value_type = Vec<Object>))]
    edus: Vec<Box<RawJsonValue>>,
}

impl IncomingTransaction {
    fn validate(&self, origin: &ServerName) -> AppResult<()> {
        if self.origin != origin {
            return Err(MatrixError::forbidden(
                "not allowed to send transactions on behalf of other servers",
                None,
            )
            .into());
        }
        if self.pdus.len() > PDU_LIMIT {
            return Err(MatrixError::forbidden(
                format!("not allowed to send more than {PDU_LIMIT} PDUs in one transaction"),
                None,
            )
            .into());
        }
        if self.edus.len() > EDU_LIMIT {
            return Err(MatrixError::forbidden(
                format!("not allowed to send more than {EDU_LIMIT} EDUs in one transaction"),
                None,
            )
            .into());
        }
        Ok(())
    }
}

fn parse_edus(edus: Vec<Box<RawJsonValue>>) -> impl Iterator<Item = Edu> {
    edus.into_iter().filter_map(|raw| {
        serde_json::from_str(raw.get())
            .map_err(|error| warn!(%error, "could not parse EDU; ignoring ephemeral update"))
            .ok()
    })
}

/// #PUT /_matrix/federation/v1/send/{txn_id}
/// Push EDUs and PDUs to this server.
#[endpoint]
async fn send_message(
    depot: &mut Depot,
    _txn_id: PathParam<OwnedTransactionId>,
    body: JsonBody<IncomingTransaction>,
) -> JsonResult<SendMessageResBody> {
    let origin = depot.origin()?;
    let body = body.into_inner();
    body.validate(origin)?;

    let txn_start_time = Instant::now();
    let resolved_map = process_pdus(&body.pdus, &body.origin, &txn_start_time).await?;
    process_edus(parse_edus(body.edus), &body.origin).await?;

    json_ok(SendMessageResBody {
        pdus: resolved_map
            .into_iter()
            .map(|(e, r)| (e, r.map_err(|e| e.to_string())))
            .collect(),
    })
}

async fn process_pdus(
    pdus: &[Box<RawJsonValue>],
    origin: &ServerName,
    txn_start_time: &Instant,
) -> AppResult<BTreeMap<OwnedEventId, AppResult<()>>> {
    let mut parsed_pdus = Vec::with_capacity(pdus.len());
    for pdu in pdus {
        parsed_pdus.push(match parse_incoming_pdu(pdu).await {
            Ok(t) => t,
            Err(e) => {
                warn!("could not parse pdu: {e}");
                continue;
            }
        });
    }
    let mut resolved_map = BTreeMap::new();
    for (event_id, value, room_id, room_version_id) in parsed_pdus {
        // crate::server::check_running()?;
        let pdu_start_time = Instant::now();
        let result = handler::process_incoming_pdu(
            origin,
            &event_id,
            &room_id,
            &room_version_id,
            value,
            true,
            false,
        )
        .await;
        debug!(
            pdu_elapsed = ?pdu_start_time.elapsed(),
            txn_elapsed = ?txn_start_time.elapsed(),
            "finished pdu {event_id}",
        );

        resolved_map.insert(event_id, result);
    }

    for (id, result) in &resolved_map {
        if let Err(e) = result
            && matches!(e, AppError::Matrix(_))
        {
            warn!("incoming pdu failed {id}: {e:?}");
        }
    }

    Ok(resolved_map)
}

async fn process_edus(edus: impl Iterator<Item = Edu>, origin: &ServerName) -> AppResult<()> {
    for edu in edus {
        match edu {
            Edu::Presence(presence) => process_edu_presence(origin, presence).await,
            Edu::Receipt(receipt) => process_edu_receipt(origin, receipt).await,
            Edu::Typing(typing) => process_edu_typing(origin, typing).await,
            Edu::DeviceListUpdate(content) => {
                process_edu_device_list_update(origin, content).await?
            }
            Edu::DirectToDevice(content) => process_edu_direct_to_device(origin, content).await,
            Edu::SigningKeyUpdate(content) => process_edu_signing_key_update(origin, content).await,
            Edu::_Custom(ref _custom) => {
                warn!("received custom/unknown edu");
            }
        }
    }
    Ok(())
}

async fn process_edu_presence(origin: &ServerName, presence: PresenceContent) {
    if !crate::config::get().presence.allow_incoming {
        return;
    }

    for update in presence.push {
        if update.user_id.server_name() != origin {
            warn!(
                %update.user_id, %origin,
                "received presence edu for user not belonging to origin"
            );
            continue;
        }

        // MSC4495: keep our view of who this user shares presence with up to date before
        // recording the transition, since that set is what limits who is shown it.
        #[cfg(feature = "unstable-msc4495")]
        if let Err(e) = track_presence_recipients(origin, &update).await {
            warn!(user_id = %update.user_id, error = %e, "failed to update presence recipients");
            continue;
        }

        crate::data::user::set_presence(
            NewDbPresence {
                user_id: update.user_id.clone(),
                stream_id: None,
                state: Some(update.presence.to_string()),
                last_active_at: Some(UnixMillis::now()),
                last_federation_update_at: None,
                last_user_sync_at: None,
                currently_active: None,
                occur_sn: None,
                updated_at: UnixMillis::now(),
                status_msg: update.status_msg.clone(),
            },
            true,
        )
        .await
        .ok();
    }
}

/// Applies an incoming update's recipient-set information ([MSC4495] inbound rules).
///
/// When the delta does not fit the view we hold, the set is emptied and a snapshot is
/// requested from the origin. Emptying first is the conservative direction: a stale wide
/// set would keep showing the user's presence to people they may have just denied, whereas
/// a stale empty set only delays presence until the snapshot arrives.
///
/// [MSC4495]: https://github.com/matrix-org/matrix-spec-proposals/pull/4495
#[cfg(feature = "unstable-msc4495")]
async fn track_presence_recipients(
    origin: &ServerName,
    update: &PresenceUpdate,
) -> crate::AppResult<()> {
    use crate::user::presence::recipients::{self, Inbound};

    let user_id = &update.user_id;
    let known = recipients::remote_set(user_id).await?;
    let known_stream_id = known.as_ref().map(|(stream_id, _)| *stream_id);

    match recipients::classify(
        known_stream_id,
        update.stream_id,
        update.prev_id,
        update.recipients.as_ref(),
    ) {
        Inbound::Legacy => {
            // A downgraded or mixed-deployment origin has returned to legacy semantics.
            // Keeping its old selective row would continue filtering through a stale set.
            recipients::clear_remote_set(user_id).await?;
        }
        Inbound::Unchanged => {}
        Inbound::Apply { stream_id, updates } => {
            let mut set = known.map(|(_, set)| set).unwrap_or_default();
            recipients::apply(&mut set, &updates);
            // As with recovered snapshots, the origin only speaks for our users' view;
            // entries naming other servers are meaningless here and would only bloat the row.
            set.retain(|recipient| recipient.server_name() == crate::config::server_name());
            recipients::store_remote_set(user_id, Some(stream_id), &set).await?;
        }
        Inbound::Resync => {
            // Marked unknown rather than stored at the position the sender claimed: a
            // later delta must not appear to apply cleanly on top of a set we know is
            // wrong, and leaving it unknown makes every further update retry the fetch
            // until one succeeds.
            recipients::schedule_resync(origin, user_id).await?;
        }
    }
    Ok(())
}

async fn process_edu_receipt(origin: &ServerName, receipt: ReceiptContent) {
    if !crate::config::get().read_receipt.allow_incoming {
        return;
    }

    for (room_id, room_updates) in receipt {
        if handler::acl_check(origin, &room_id).await.is_err() {
            warn!(
                %origin, %room_id,
                "received read receipt edu from ACL'd server"
            );
            continue;
        }

        for (user_id, user_updates) in room_updates.read {
            // if user_id.server_name() != origin {
            //     warn!(
            //         %user_id, %origin,
            //         "received read receipt edu for user not belonging to origin"
            //     );
            //     continue;
            // }

            if room::joined_users(&room_id, None)
                .await
                .unwrap_or_default()
                .iter()
                .any(|member| member.server_name() == user_id.server_name())
            {
                for event_id in &user_updates.event_ids {
                    let user_receipts =
                        BTreeMap::from([(user_id.clone(), user_updates.data.clone())]);
                    let receipts = BTreeMap::from([(ReceiptType::Read, user_receipts)]);
                    let receipt_content = BTreeMap::from([(event_id.to_owned(), receipts)]);
                    let event = ReceiptEvent {
                        content: ReceiptEventContent(receipt_content),
                        room_id: room_id.clone(),
                    };

                    let _ = room::receipt::update_read(&user_id, &room_id, &event, false).await;
                }
            } else {
                warn!(
                    %user_id, %room_id, %origin,
                    "received read receipt edu from server who does not have a member in the room",
                );
                continue;
            }
        }
    }
}

async fn process_edu_typing(origin: &ServerName, typing: TypingContent) {
    if !crate::config::get().typing.allow_incoming {
        return;
    }

    if typing.user_id.server_name() != origin {
        warn!(
            %typing.user_id, %origin,
            "received typing edu for user not belonging to origin"
        );
        return;
    }

    if handler::acl_check(typing.user_id.server_name(), &typing.room_id)
        .await
        .is_err()
    {
        warn!(
            %typing.user_id, %typing.room_id, %origin,
            "received typing edu for ACL'd user's server"
        );
        return;
    }

    if room::user::is_joined(&typing.user_id, &typing.room_id)
        .await
        .unwrap_or(false)
    {
        if typing.typing {
            let timeout = UnixMillis::now().get().saturating_add(
                crate::config::get()
                    .typing
                    .federation_timeout
                    .saturating_mul(1000),
            );
            let _ =
                room::typing::add_typing(&typing.user_id, &typing.room_id, timeout, false).await;
        } else {
            let _ = room::typing::remove_typing(&typing.user_id, &typing.room_id, false).await;
        }
    } else {
        warn!(
            %typing.user_id, %typing.room_id, %origin,
            "received typing edu for user not in room"
        );
    }
}

async fn process_edu_device_list_update(
    origin: &ServerName,
    content: DeviceListUpdateContent,
) -> AppResult<()> {
    let DeviceListUpdateContent {
        user_id, stream_id, ..
    } = content;

    if user_id.server_name() != origin {
        warn!(
            %user_id, %origin,
            "received device list update edu for user not belonging to origin"
        );
        return Ok(());
    }
    crate::data::user::key::apply_remote_device_list_version(
        &user_id,
        stream_id,
        crate::config::server_name(),
    )
    .await?;
    Ok(())
}

async fn process_edu_direct_to_device(origin: &ServerName, content: DirectDeviceContent) {
    let DirectDeviceContent {
        sender,
        ev_type,
        message_id,
        messages,
    } = content;

    if sender.server_name() != origin {
        warn!(
            %sender, %origin,
            "received direct to device edu for user not belonging to origin"
        );
        return;
    }

    // Check if this is a new transaction id
    if crate::transaction_id::txn_id_exists(&message_id, &sender, None)
        .await
        .unwrap_or_default()
    {
        return;
    }

    for (target_user_id, map) in &messages {
        for (target_device_id_maybe, event) in map {
            let Ok(event) = event
                .deserialize_as()
                .map_err(|e| error!("to-device event is invalid: {e}"))
            else {
                continue;
            };

            let ev_type = ev_type.to_string();
            match target_device_id_maybe {
                DeviceIdOrAllDevices::DeviceId(target_device_id) => {
                    let _ = data::user::device::add_to_device_event(
                        &sender,
                        target_user_id,
                        target_device_id,
                        &ev_type,
                        event,
                    )
                    .await;
                }

                DeviceIdOrAllDevices::AllDevices => {
                    let (sender, ev_type, event) = (&sender, &ev_type, &event);
                    for target_device_id in data::user::all_to_device_target_ids(target_user_id)
                        .await
                        .unwrap_or_default()
                        .iter()
                    {
                        let _ = data::user::device::add_to_device_event(
                            sender,
                            target_user_id,
                            target_device_id,
                            ev_type,
                            event.clone(),
                        )
                        .await;
                    }
                }
            }
        }
    }

    // Save transaction id with empty data
    let _ = crate::transaction_id::add_txn_id(&message_id, &sender, None, None, None).await;
}

async fn process_edu_signing_key_update(origin: &ServerName, content: SigningKeyUpdateContent) {
    let SigningKeyUpdateContent {
        user_id,
        master_key,
        self_signing_key,
    } = content;

    if user_id.server_name() != origin {
        warn!(
            %user_id, %origin,
            "received signing key update EDU from server that does not belong to user's server"
        );
        return;
    }

    if let Some(master_key) = master_key {
        let _ = crate::user::add_cross_signing_keys(
            &user_id,
            &master_key,
            &self_signing_key,
            &None,
            true,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn malformed_edu_does_not_discard_persistent_or_valid_ephemeral_events() {
        let pdu = json!({"type": "m.room.message", "content": {"body": "keep me"}});
        let body: IncomingTransaction = serde_json::from_value(json!({
            "origin": "remote.example",
            "origin_server_ts": 1,
            "pdus": [pdu],
            "edus": [
                {"edu_type": "m.presence", "content": {"push": [
                    {"user_id": "@alice:remote.example", "presence": "online"}
                ]}},
                {"edu_type": "m.typing", "content": {
                    "room_id": "!room:remote.example",
                    "user_id": "@alice:remote.example", "typing": true
                }},
                null
            ]
        }))
        .unwrap();
        body.validate(&body.origin).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body.pdus[0].get()).unwrap(),
            pdu
        );
        let edus: Vec<_> = parse_edus(body.edus).collect();
        assert!(matches!(edus.as_slice(), [Edu::Typing(content)] if content.typing));
    }

    #[test]
    fn malformed_edus_still_count_towards_transaction_limit() {
        let body: IncomingTransaction = serde_json::from_value(json!({
            "origin": "remote.example",
            "origin_server_ts": 1,
            "edus": vec![json!(null); EDU_LIMIT + 1]
        }))
        .unwrap();
        assert!(body.validate(&body.origin).is_err());
        assert!(
            body.validate(&ServerName::parse("another.example").unwrap())
                .is_err()
        );
    }
}

#[cfg(test)]
mod device_version_tests {
    use diesel::{ExpressionMethods, QueryDsl};
    use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;
    use crate::core::federation::discovery::{ServerSigningKeys, VerifyKey};
    use crate::core::serde::CanonicalJsonObject;
    use crate::core::signatures::{Ed25519KeyPair, sign_json};
    use crate::data::schema::{e2e_key_changes, remote_device_list_versions, room_users, rooms};

    const LOCAL: &str = "device-versions.example";
    const REMOTE: &str = "remote-devices.example";
    const TOKEN: &str = "device-version-alice";

    async fn send_update(
        service: &Service,
        key: &Ed25519KeyPair,
        txn: &str,
        stream: u64,
        device: &str,
    ) -> (StatusCode, Value) {
        let path = format!("/_matrix/federation/v1/send/{txn}");
        let body = json!({"origin":REMOTE,"origin_server_ts":1,"pdus":[],"edus":[{
            "edu_type":"m.device_list_update","content":{
                "user_id":format!("@bob:{REMOTE}"),"device_id":device,
                "stream_id":stream,"prev_id":[]
            }
        }]});
        let mut signed: CanonicalJsonObject = serde_json::from_value(json!({
            "method":"PUT","uri":path,"origin":REMOTE,"destination":LOCAL,"content":body
        }))
        .unwrap();
        sign_json(REMOTE, key, &mut signed).unwrap();
        let signed = serde_json::to_value(signed).unwrap();
        let signature = signed["signatures"][REMOTE]["ed25519:versions"]
            .as_str()
            .unwrap();
        let mut response = TestClient::put(format!("http://localhost{path}"))
            .json(&body)
            .add_header("authorization", format!(
                "X-Matrix origin=\"{REMOTE}\",destination=\"{LOCAL}\",key=\"ed25519:versions\",sig=\"{signature}\""
            ), true)
            .send(service).await;
        (
            response.status_code.unwrap(),
            response.take_json().await.unwrap(),
        )
    }

    async fn sync(service: &Service, since: Option<&str>) -> Value {
        let since = since
            .map(|since| format!("&since={since}"))
            .unwrap_or_default();
        let request = TestClient::get(format!(
            "http://localhost/_matrix/client/v3/sync?timeout=0{since}"
        ))
        .add_header("authorization", format!("Bearer {TOKEN}"), true);
        let mut response = request.send(service).await;
        let result: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{result}");
        result
    }

    async fn join_current_membership(room: &RoomId, user: &UserId) {
        diesel::insert_into(room_users::table)
            .values(crate::data::room::NewDbRoomUser {
                event_id: format!("$device-{}", user.localpart()).try_into().unwrap(),
                event_sn: 1,
                room_id: room.to_owned(),
                room_server_id: Some(LOCAL.try_into().unwrap()),
                user_id: user.to_owned(),
                user_server_id: user.server_name().to_owned(),
                sender_id: user.to_owned(),
                membership: "join".into(),
                forgotten: false,
                display_name: None,
                avatar_url: None,
                state_data: None,
                created_at: UnixMillis::now(),
            })
            .execute(&mut crate::data::connect().await.unwrap())
            .await
            .unwrap();
    }

    async fn setup() -> (
        Service,
        Ed25519KeyPair,
        OwnedUserId,
        OwnedUserId,
        OwnedRoomId,
    ) {
        crate::test_database::init();
        crate::config::CONFIG
            .set(
                serde_json::from_value(json!({
                    "server_name":LOCAL,"db":{"url":"unused-test-config"},
                    "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
                }))
                .unwrap(),
            )
            .expect("run this HTTP regression alone");
        let alice = UserId::parse(format!("@alice:{LOCAL}")).unwrap();
        let bob = UserId::parse(format!("@bob:{REMOTE}")).unwrap();
        crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: alice.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: alice.localpart().into(),
            server_name: alice.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        crate::data::user::device::create_device(&alice, "ALICE".into(), TOKEN, None, None)
            .await
            .unwrap();
        let room: OwnedRoomId = format!("!device-versions:{LOCAL}").try_into().unwrap();
        let mut conn = crate::data::connect().await.unwrap();
        diesel::insert_into(rooms::table)
            .values(crate::data::room::NewDbRoom {
                id: room.clone(),
                version: "11".into(),
                is_public: false,
                min_depth: 0,
                has_auth_chain_index: false,
                created_at: UnixMillis::now(),
            })
            .execute(&mut conn)
            .await
            .unwrap();
        // Device tracking is scoped by the current joined membership rows. No
        // room event is involved in accepting or replaying a device-list EDU.
        drop(conn);
        join_current_membership(&room, &alice).await;
        let key = Ed25519KeyPair::from_der(&Ed25519KeyPair::generate().unwrap(), "versions".into())
            .unwrap();
        let mut keys = ServerSigningKeys::new(
            REMOTE.try_into().unwrap(),
            UnixMillis(UnixMillis::now().0 + 86_400_000),
        );
        keys.verify_keys.insert(
            "ed25519:versions".try_into().unwrap(),
            VerifyKey::from_bytes(key.public_key().to_vec()),
        );
        crate::server_key::add_signing_keys(keys).await.unwrap();
        let service = Service::new(crate::routing::matrix());
        (service, key, alice, bob, room)
    }

    async fn run() {
        let (service, key, alice, bob, room) = setup().await;
        let before = crate::data::curr_sn().await.unwrap();
        assert_eq!(
            send_update(&service, &key, "not-yet-shared", 0, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            crate::data::curr_sn().await.unwrap(),
            before,
            "untracked user consumed a version"
        );
        join_current_membership(&room, &bob).await;
        let initial = sync(&service, None).await;
        let response = send_update(&service, &key, "first", 0, "BOB").await;
        assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
        let acknowledged = sync(&service, initial["next_batch"].as_str()).await;
        assert_eq!(acknowledged["device_lists"]["changed"], json!([bob]));

        // A placeholder and the explicit device can describe the same opaque
        // version in separate transactions. Replaying it after acknowledgement
        // must not pollute Alice's own new-device change with Bob again.
        let before = crate::data::curr_sn().await.unwrap();
        let response = send_update(&service, &key, "duplicate", 0, "placeholder").await;
        assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
        assert_eq!(
            crate::data::curr_sn().await.unwrap(),
            before,
            "duplicate EDU allocated a new stream position"
        );
        crate::data::user::device::create_device(
            &alice,
            "SECOND".into(),
            "device-version-second",
            None,
            None,
        )
        .await
        .unwrap();
        let mut response = TestClient::post("http://localhost/_matrix/client/v3/keys/upload")
            .add_header("authorization", "Bearer device-version-second", true)
            .json(&json!({"device_keys":{"user_id":alice,"device_id":"SECOND",
                "algorithms":["m.olm.v1.curve25519-aes-sha2"],"keys":{},"signatures":{}}}))
            .send(&service)
            .await;
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
        let own_change = sync(&service, acknowledged["next_batch"].as_str()).await;
        assert_eq!(own_change["device_lists"]["changed"], json!([alice]));

        let before = crate::data::curr_sn().await.unwrap();
        let transaction_ids = (0..8)
            .map(|i| format!("concurrent-{i}"))
            .collect::<Vec<_>>();
        let sends = transaction_ids
            .iter()
            .map(|txn| send_update(&service, &key, txn, 37, "BOB"));
        for response in futures_util::future::join_all(sends).await {
            assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
        }
        assert_eq!(
            crate::data::curr_sn().await.unwrap(),
            before + 1,
            "concurrent duplicate deliveries invalidated more than once"
        );

        // IDs are opaque, not ordered: a new lower value is a new version too.
        let before = crate::data::curr_sn().await.unwrap();
        assert_eq!(
            send_update(&service, &key, "lower-version", 12, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);

        let mut conn = crate::data::connect().await.unwrap();
        conn.batch_execute("CREATE FUNCTION fail_device_version() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.user_id = '@bob:remote-devices.example' THEN RAISE EXCEPTION 'injected device marker failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER fail_device_version BEFORE INSERT ON e2e_key_changes FOR EACH ROW EXECUTE FUNCTION fail_device_version();").await.unwrap();
        drop(conn);
        let response = send_update(&service, &key, "failure", 55, "BOB").await;
        assert_eq!(
            response.0,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{:?}",
            response.1
        );
        let mut conn = crate::data::connect().await.unwrap();
        conn.batch_execute("DROP TRIGGER fail_device_version ON e2e_key_changes; DROP FUNCTION fail_device_version();").await.unwrap();
        drop(conn);
        let before = crate::data::curr_sn().await.unwrap();
        assert_eq!(
            send_update(&service, &key, "retry-after-failure", 55, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            crate::data::curr_sn().await.unwrap(),
            before + 1,
            "failed invalidation incorrectly remembered the version"
        );

        // The decoded content type permits all u64 values, while the transport's
        // canonical-JSON signature range is narrower. Exercise the handler's
        // persistence boundary directly for the maximum value, without a cast.
        let large: DeviceListUpdateContent = serde_json::from_value(json!({
            "user_id":bob,"device_id":"BOB","stream_id":u64::MAX,"prev_id":[]
        }))
        .unwrap();
        let before = crate::data::curr_sn().await.unwrap();
        let remote = ServerName::parse(REMOTE).unwrap();
        process_edu_device_list_update(&remote, large.clone())
            .await
            .unwrap();
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);
        process_edu_device_list_update(&remote, large)
            .await
            .unwrap();
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);

        for stream in 1_000..1_260 {
            let response =
                send_update(&service, &key, &format!("window-{stream}"), stream, "BOB").await;
            assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
        }
        let count: i64 = remote_device_list_versions::table
            .filter(remote_device_list_versions::user_id.eq(&bob))
            .count()
            .get_result(&mut crate::data::connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(
            count, 256,
            "remote version history grew beyond its durable window"
        );
        let before = crate::data::curr_sn().await.unwrap();
        assert_eq!(
            send_update(&service, &key, "window-recent", 1_259, "placeholder")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(crate::data::curr_sn().await.unwrap(), before);
        // An evicted low ID is conservatively invalidated and retained as the
        // newest receipt, proving eviction does not sort opaque remote IDs.
        assert_eq!(
            send_update(&service, &key, "window-evicted", 1_000, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);
        let retained: bool = diesel::select(diesel::dsl::exists(
            remote_device_list_versions::table
                .filter(remote_device_list_versions::user_id.eq(&bob))
                .filter(remote_device_list_versions::stream_id.eq("1000")),
        ))
        .get_result(&mut crate::data::connect().await.unwrap())
        .await
        .unwrap();
        assert!(retained);
        diesel::delete(room_users::table.filter(room_users::user_id.eq(&alice)))
            .execute(&mut crate::data::connect().await.unwrap())
            .await
            .unwrap();
        let before = crate::data::curr_sn().await.unwrap();
        assert_eq!(
            send_update(&service, &key, "no-local-member", 1_259, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(crate::data::curr_sn().await.unwrap(), before);
        let count: i64 = remote_device_list_versions::table
            .filter(remote_device_list_versions::user_id.eq(&bob))
            .count()
            .get_result(&mut crate::data::connect().await.unwrap())
            .await
            .unwrap();
        assert_eq!(count, 0, "unshared user's retained window was not cleared");
        join_current_membership(&room, &alice).await;
        assert_eq!(
            send_update(&service, &key, "shared-again", 1_259, "BOB")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);

        // Each atomic membership transition has exactly one joined side. A
        // single statement must never invent shared membership by combining
        // remote and local rows from different READ COMMITTED snapshots.
        let mut conn = crate::data::connect().await.unwrap();
        diesel::sql_query("UPDATE room_users SET membership = CASE WHEN user_id = $1 THEN 'leave' ELSE 'join' END WHERE user_id IN ($1, $2) AND room_id = $3")
            .bind::<diesel::sql_types::Text,_>(alice.as_str())
            .bind::<diesel::sql_types::Text,_>(bob.as_str())
            .bind::<diesel::sql_types::Text,_>(room.as_str())
            .execute(&mut conn).await.unwrap();
        let before = crate::data::curr_sn().await.unwrap();
        let writers = async {
            for phase in 0..128 {
                diesel::sql_query("UPDATE room_users SET membership = CASE WHEN user_id = $1 THEN CASE WHEN $3 THEN 'join' ELSE 'leave' END ELSE CASE WHEN $3 THEN 'leave' ELSE 'join' END END WHERE user_id IN ($1, $2) AND room_id = $4")
                    .bind::<diesel::sql_types::Text,_>(alice.as_str())
                    .bind::<diesel::sql_types::Text,_>(bob.as_str())
                    .bind::<diesel::sql_types::Bool,_>(phase%2==0)
                    .bind::<diesel::sql_types::Text,_>(room.as_str())
                    .execute(&mut conn).await.unwrap();
                tokio::task::yield_now().await;
            }
        };
        let readers = async {
            for version in 2_000..2_064 {
                let response = send_update(
                    &service,
                    &key,
                    &format!("snapshot-{version}"),
                    version,
                    "BOB",
                )
                .await;
                assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
            }
        };
        let ((), ()) = tokio::join!(writers, readers);
        assert_eq!(
            crate::data::curr_sn().await.unwrap(),
            before,
            "membership snapshots invented a shared room"
        );
        let cached: i64 = remote_device_list_versions::table
            .filter(remote_device_list_versions::user_id.eq(&bob))
            .count()
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            cached, 0,
            "a never-shared membership pair consumed remote versions"
        );
    }

    async fn many_rooms() {
        let (service, key, _alice, bob, _room) = setup().await;
        let mut conn = crate::data::connect().await.unwrap();
        diesel::sql_query("INSERT INTO rooms (id, version, is_public, min_depth, has_auth_chain_index, created_at) SELECT '!device-many-' || number || ':' || $1, '11', false, 0, false, 0 FROM generate_series(1, 16500) AS seed(number)")
            .bind::<diesel::sql_types::Text,_>(LOCAL)
            .execute(&mut conn).await.unwrap();
        diesel::sql_query("INSERT INTO room_users (event_id, event_sn, room_id, room_server_id, user_id, user_server_id, sender_id, membership, forgotten, created_at) SELECT '$many-' || number || '-' || member.localpart, 1, '!device-many-' || number || ':' || $1, $1, member.user_id, member.server_id, member.user_id, 'join', false, 0 FROM generate_series(1, 16500) AS seed(number) CROSS JOIN (VALUES ($2, $1, 'alice'), ($3, $4, 'bob')) AS member(user_id, server_id, localpart)")
            .bind::<diesel::sql_types::Text,_>(LOCAL)
            .bind::<diesel::sql_types::Text,_>(format!("@alice:{LOCAL}"))
            .bind::<diesel::sql_types::Text,_>(bob.as_str())
            .bind::<diesel::sql_types::Text,_>(REMOTE)
            .execute(&mut conn).await.unwrap();
        drop(conn);
        let before = crate::data::curr_sn().await.unwrap();
        let response = send_update(&service, &key, "many-shared-rooms", 1, "BOB").await;
        assert_eq!(response.0, StatusCode::OK, "{:?}", response.1);
        assert_eq!(crate::data::curr_sn().await.unwrap(), before + 1);
        let mut conn = crate::data::connect().await.unwrap();
        let count: i64 = e2e_key_changes::table
            .filter(e2e_key_changes::user_id.eq(&bob))
            .count()
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            count, 16_501,
            "a shared room or the global marker was dropped"
        );
        let positions: Vec<i64> = e2e_key_changes::table
            .filter(e2e_key_changes::user_id.eq(&bob))
            .select(e2e_key_changes::occur_sn)
            .distinct()
            .load(&mut conn)
            .await
            .unwrap();
        assert_eq!(positions, vec![before + 1]);
        let global: i64 = e2e_key_changes::table
            .filter(e2e_key_changes::user_id.eq(&bob))
            .filter(e2e_key_changes::room_id.is_null())
            .count()
            .get_result(&mut conn)
            .await
            .unwrap();
        assert_eq!(global, 1);
    }

    #[test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    fn http_remote_device_versions_are_durable_and_idempotent() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(run())
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
    fn http_remote_device_versions_cover_many_shared_rooms() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(many_rooms())
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
