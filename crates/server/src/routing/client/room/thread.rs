use salvo::prelude::*;

use crate::core::client::room::{ThreadsReqArgs, ThreadsResBody};
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

/// #GET /_matrix/client/r0/rooms/{room_id}/threads
#[endpoint]
pub(super) async fn list_threads(
    _aa: AuthArgs,
    args: ThreadsReqArgs,
    depot: &mut Depot,
) -> JsonResult<ThreadsResBody> {
    let authed = depot.authed_info()?;

    if args.limit == Some(0) {
        return Err(MatrixError::invalid_param("Thread limit must be greater than zero.").into());
    }

    // Use limit or else 10, with maximum 100
    let limit = args
        .limit
        .and_then(|l| l.try_into().ok())
        .unwrap_or(10)
        .min(100);

    let (threads, next_batch) = crate::room::thread::get_threads(
        &args.room_id,
        authed.user_id(),
        &args.include,
        limit,
        args.from.as_deref(),
    )
    .await?;

    json_ok(ThreadsResBody {
        chunk: threads
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event_for(authed.user_id(), authed.device_id()))
            .collect(),
        next_batch,
    })
}

#[cfg(test)]
mod private_cursor_tests {
    use diesel::prelude::*;
    use horme::RunQueryDsl;
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;
    use crate::core::{EventId, RoomId, UserId};
    use crate::data;

    async fn request(service: &Service, token: &str, request: RequestBuilder) -> Value {
        let mut response = request
            .add_header("authorization", format!("Bearer {token}"), true)
            .send(service)
            .await;
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
        body
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
    fn database_thread_cursors_never_expose_private_or_prejoin_root_ids() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(run());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn run() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"thread-privacy.example", "db":{"url":"unused"}
            }))
            .unwrap()
        });
        let mut users = Vec::new();
        for name in ["owner", "reader", "outsider"] {
            let user =
                UserId::parse(format!("@q16{name}:{}", crate::config::server_name())).unwrap();
            data::user::create_user(&data::user::NewDbUser {
                id: user.clone(),
                localpart: user.localpart().into(),
                server_name: user.server_name().to_owned(),
                is_local: true,
                ty: None,
                is_admin: true,
                is_guest: false,
                appservice_id: None,
                created_at: crate::core::UnixMillis::now(),
            })
            .await
            .unwrap();
            data::user::device::create_device(&user, "Q16".into(), name, None, None)
                .await
                .unwrap();
            users.push(user);
        }
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(
                    Router::with_path("createRoom").post(crate::routing::client::room::create_room),
                )
                .push(crate::routing::client::room::authed_router()),
        );
        let created = request(&service, "owner", TestClient::post("http://localhost/createRoom").json(&json!({
            "room_version":"11", "preset":"private_chat", "invite":[users[1]],
            "initial_state":[{"type":"m.room.history_visibility", "state_key":"", "content":{"history_visibility":"joined"}}]
        }))).await;
        let room = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
        let mut roots = Vec::new();
        for name in ["hidden-a", "hidden-b", "visible-a", "visible-b"] {
            if name == "visible-a" {
                request(
                    &service,
                    "reader",
                    TestClient::post(format!("http://localhost/rooms/{room}/join"))
                        .json(&json!({})),
                )
                .await;
            }
            let sent = request(
                &service,
                "owner",
                TestClient::put(format!(
                    "http://localhost/rooms/{room}/send/m.room.message/{name}"
                ))
                .json(&json!({"msgtype":"m.text", "body":name})),
            )
            .await;
            let id = EventId::parse(sent["event_id"].as_str().unwrap()).unwrap();
            let pdu = crate::room::timeline::get_pdu(&id).await.unwrap();
            // Place hidden roots first by activity to exercise visibility-aware scanning.
            let activity = if name.starts_with("hidden") {
                10000 + roots.len() as i64
            } else {
                pdu.event_sn
            };
            data::room::upsert_thread(data::room::DbThread {
                event_id: id.clone(),
                event_sn: pdu.event_sn,
                room_id: room.clone(),
                last_id: id.clone(),
                last_sn: activity,
            })
            .await
            .unwrap();
            roots.push(id);
        }
        let outsider = request(
            &service,
            "outsider",
            TestClient::get(format!("http://localhost/rooms/{room}/threads?limit=1")),
        )
        .await;
        assert!(outsider["chunk"].as_array().unwrap().is_empty());
        assert!(outsider.get("next_batch").is_none_or(Value::is_null));
        let first = request(
            &service,
            "reader",
            TestClient::get(format!("http://localhost/rooms/{room}/threads?limit=1")),
        )
        .await;
        assert_eq!(first["chunk"][0]["event_id"], roots[3].as_str());
        let token = first["next_batch"]
            .as_str()
            .expect("another visible root remains");
        let cursor = crate::room::thread::parse_cursor(
            token,
            &room,
            &users[1],
            &crate::core::client::room::IncludeThreads::All,
        )
        .await
        .unwrap();
        assert_eq!(cursor.event_id, roots[3]);
        assert!(!roots[..2].contains(&cursor.event_id));
        let second = request(
            &service,
            "reader",
            TestClient::get(format!(
                "http://localhost/rooms/{room}/threads?limit=1&from={token}"
            )),
        )
        .await;
        assert_eq!(second["chunk"][0]["event_id"], roots[2].as_str());
        assert!(second.get("next_batch").is_none_or(Value::is_null));

        // A legacy visible anchor describes historical activity. New replies
        // must not invalidate it merely because threads.last_sn has advanced.
        let legacy_for = |activity_sn| {
            format!(
                "t1.{}",
                base64::Engine::encode(
                    &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                    serde_json::to_vec(
                        &json!({"room_id":room, "last_sn":activity_sn, "event_id":roots[3]})
                    )
                    .unwrap()
                )
            )
        };
        let mut legacy = None;
        for number in 1..=2 {
            let sent = request(
                &service,
                "owner",
                TestClient::put(format!(
                    "http://localhost/rooms/{room}/send/m.room.message/new-reply-{number}"
                ))
                .json(
                    &json!({"msgtype":"m.text", "body":"new activity", "m.relates_to":{
                        "rel_type":"m.thread", "event_id":roots[3], "is_falling_back":true,
                        "m.in_reply_to":{"event_id":roots[3]}
                    }}),
                ),
            )
            .await;
            if number == 1 {
                let id = EventId::parse(sent["event_id"].as_str().unwrap()).unwrap();
                let reply = crate::room::timeline::get_pdu(&id).await.unwrap();
                legacy = Some(legacy_for(reply.event_sn));
                // Historical installations need not have backfilled this field.
                use crate::data::schema::event_points;
                diesel::update(event_points::table.find(&id))
                    .set(event_points::thread_id.eq(None::<String>))
                    .execute(&mut crate::data::db())
                    .await
                    .unwrap();
            }
        }
        let legacy = legacy.unwrap();
        let resumed = request(
            &service,
            "reader",
            TestClient::get(format!(
                "http://localhost/rooms/{room}/threads?limit=1&from={legacy}"
            )),
        )
        .await;
        assert_eq!(resumed["chunk"][0]["event_id"], roots[2].as_str());
        let unrelated = crate::room::timeline::get_pdu(&roots[2]).await.unwrap();
        let invalid_legacy = legacy_for(unrelated.event_sn);
        let mut rejected = TestClient::get(format!(
            "http://localhost/rooms/{room}/threads?limit=1&from={invalid_legacy}"
        ))
        .add_header("authorization", "Bearer reader", true)
        .send(&service)
        .await;
        let rejected_body: Value = rejected.take_json().await.unwrap();
        assert_eq!(
            rejected.status_code,
            Some(StatusCode::BAD_REQUEST),
            "{rejected_body}"
        );

        // A nonmember may read a historical world-readable root. More than a
        // request budget of newer private roots must require resumable, private
        // scanning instead of either an unbounded scan or losing that root.
        let created = request(&service, "owner", TestClient::post("http://localhost/createRoom")
            .json(&json!({"room_version":"11", "preset":"private_chat", "initial_state":[{
                "type":"m.room.history_visibility", "state_key":"", "content":{"history_visibility":"world_readable"}
            }]}))).await;
        let scan_room = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
        let visible = request(
            &service,
            "owner",
            TestClient::put(format!(
                "http://localhost/rooms/{scan_room}/send/m.room.message/historical-visible"
            ))
            .json(&json!({"msgtype":"m.text", "body":"historically world readable"})),
        )
        .await;
        let visible = EventId::parse(visible["event_id"].as_str().unwrap()).unwrap();
        let pdu = crate::room::timeline::get_pdu(&visible).await.unwrap();
        data::room::upsert_thread(data::room::DbThread {
            event_id: visible.clone(),
            event_sn: pdu.event_sn,
            room_id: scan_room.clone(),
            last_id: visible.clone(),
            last_sn: pdu.event_sn,
        })
        .await
        .unwrap();
        request(
            &service,
            "owner",
            TestClient::put(format!(
                "http://localhost/rooms/{scan_room}/state/m.room.history_visibility"
            ))
            .json(&json!({"history_visibility":"joined"})),
        )
        .await;
        let mut hidden = Vec::new();
        let message_rate = crate::config::get().rc_message.per_second;
        for number in 0..513 {
            // Prepare the large history at the configured send rate, keeping
            // the real message limiter enabled throughout this HTTP fixture.
            if message_rate > 0.0 {
                tokio::time::sleep(std::time::Duration::from_secs_f64(1.1 / message_rate)).await;
            }
            let sent = request(
                &service,
                "owner",
                TestClient::put(format!(
                    "http://localhost/rooms/{scan_room}/send/m.room.message/hidden-{number}"
                ))
                .json(&json!({"msgtype":"m.text", "body":"private root"})),
            )
            .await;
            let id = EventId::parse(sent["event_id"].as_str().unwrap()).unwrap();
            let pdu = crate::room::timeline::get_pdu(&id).await.unwrap();
            data::room::upsert_thread(data::room::DbThread {
                event_id: id.clone(),
                event_sn: pdu.event_sn,
                room_id: scan_room.clone(),
                last_id: id.clone(),
                last_sn: pdu.event_sn,
            })
            .await
            .unwrap();
            hidden.push(id);
        }
        let first = request(
            &service,
            "outsider",
            TestClient::get(format!(
                "http://localhost/rooms/{scan_room}/threads?limit=1"
            )),
        )
        .await;
        assert!(
            first["chunk"].as_array().unwrap().is_empty(),
            "scan work must stop at its request budget: {first}"
        );
        let continuation = first["next_batch"]
            .as_str()
            .expect("hidden-only page must remain resumable");
        let decoded = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            continuation.strip_prefix("t2.").unwrap(),
        )
        .unwrap();
        assert!(!hidden.iter().any(|id| {
            decoded
                .windows(id.as_str().len())
                .any(|bytes| bytes == id.as_bytes())
        }));
        let cursor = crate::room::thread::parse_cursor(
            continuation,
            &scan_room,
            &users[2],
            &crate::core::client::room::IncludeThreads::All,
        )
        .await
        .unwrap();
        assert_eq!(
            cursor.event_id, hidden[1],
            "exactly 512 candidate roots were inspected"
        );
        for (token, target_room, include) in [
            ("owner", &scan_room, "all"),
            ("outsider", &room, "all"),
            ("outsider", &scan_room, "participated"),
        ] {
            let mut response = TestClient::get(format!(
                "http://localhost/rooms/{target_room}/threads?limit=1&include={include}&from={continuation}"
            )).add_header("authorization",format!("Bearer {token}"),true).send(&service).await;
            let body: Value = response.take_json().await.unwrap();
            assert_eq!(
                response.status_code,
                Some(StatusCode::BAD_REQUEST),
                "context replay must fail: {body}"
            );
        }
        let second = request(
            &service,
            "outsider",
            TestClient::get(format!(
                "http://localhost/rooms/{scan_room}/threads?limit=1&from={continuation}"
            )),
        )
        .await;
        assert_eq!(second["chunk"][0]["event_id"], visible.as_str());
        assert!(second.get("next_batch").is_none_or(Value::is_null));
    }
}

#[cfg(test)]
mod participation_tests {
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;

    async fn request(service: &Service, token: &str, builder: RequestBuilder) -> Value {
        let builder = builder.add_header("authorization", format!("Bearer {token}"), true);
        let router = service.router();
        let mut response = tokio::spawn(async move { builder.send(router).await })
            .await
            .unwrap();
        let value: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{value}");
        value
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
    fn database_thread_participation_is_per_user_and_filtered_before_pagination() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(run());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn run() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"threads.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        for (name, token) in [
            ("threada", "thread-a"),
            ("threadb", "thread-b"),
            ("threadc", "thread-c"),
        ] {
            let user =
                crate::core::UserId::parse(format!("@{name}:{}", crate::config::server_name()))
                    .unwrap();
            crate::data::user::create_user(&crate::data::user::NewDbUser {
                id: user.clone(),
                ty: None,
                is_admin: true,
                is_guest: false,
                is_local: true,
                localpart: name.into(),
                server_name: user.server_name().to_owned(),
                appservice_id: None,
                created_at: crate::core::UnixMillis::now(),
            })
            .await
            .unwrap();
            crate::data::user::device::create_device(
                &user,
                "THREAD_TEST".into(),
                token,
                None,
                None,
            )
            .await
            .unwrap();
        }
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(
                    Router::with_path("createRoom").post(crate::routing::client::room::create_room),
                )
                .push(crate::routing::client::room::authed_router()),
        );
        let created = request(
            &service,
            "thread-a",
            TestClient::post("http://localhost/createRoom")
                .json(&json!({"room_version":"11","preset":"public_chat"})),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        for token in ["thread-b", "thread-c"] {
            request(
                &service,
                token,
                TestClient::post(format!("http://localhost/rooms/{room}/join")).json(&json!({})),
            )
            .await;
        }
        let mut roots = Vec::new();
        for (token, txn) in [("thread-a", "root-a"), ("thread-b", "root-b")] {
            let sent = request(
                &service,
                token,
                TestClient::put(format!(
                    "http://localhost/rooms/{room}/send/m.room.message/{txn}"
                ))
                .json(&json!({"msgtype":"m.text","body":txn})),
            )
            .await;
            roots.push(sent["event_id"].as_str().unwrap().to_owned());
        }
        for (index, root) in roots.iter().enumerate() {
            request(&service,"thread-b",TestClient::put(format!("http://localhost/rooms/{room}/send/m.room.message/reply-{index}"))
                .json(&json!({"msgtype":"m.text","body":"reply","m.relates_to":{"rel_type":"m.thread","event_id":root,
                    "is_falling_back":true,"m.in_reply_to":{"event_id":root}}}))).await;
        }
        for (token, expected) in [
            ("thread-a", vec![roots[0].clone()]),
            ("thread-b", vec![roots[1].clone(), roots[0].clone()]),
            ("thread-c", vec![]),
        ] {
            let all = request(
                &service,
                token,
                TestClient::get(format!("http://localhost/rooms/{room}/threads?include=all")),
            )
            .await;
            assert_eq!(all["chunk"].as_array().unwrap().len(), 2);
            for event in all["chunk"].as_array().unwrap() {
                let id = event["event_id"].as_str().unwrap();
                let participated = expected.iter().any(|root| root == id);
                assert_eq!(
                    event["unsigned"]["m.relations"]["m.thread"]["current_user_participated"],
                    participated,
                    "{token}: {event}"
                );
                // The same personalized value must be present through /event,
                // rather than being a special case of the thread-list handler.
                let single = request(
                    &service,
                    token,
                    TestClient::get(format!("http://localhost/rooms/{room}/event/{id}")),
                )
                .await;
                assert_eq!(
                    single["unsigned"]["m.relations"]["m.thread"]["current_user_participated"],
                    participated
                );
            }
            let mut from = String::new();
            let mut actual = Vec::new();
            loop {
                let page = request(
                    &service,
                    token,
                    TestClient::get(format!(
                        "http://localhost/rooms/{room}/threads?include=participated&limit=1{from}"
                    )),
                )
                .await;
                actual.extend(
                    page["chunk"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|event| event["event_id"].as_str().unwrap().to_owned()),
                );
                if let Some(next) = page["next_batch"].as_str() {
                    from = format!("&from={next}");
                } else {
                    break;
                }
                assert!(actual.len() <= 2);
            }
            assert_eq!(actual, expected);
        }
    }
}

#[cfg(test)]
mod summary_tests {
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;

    async fn request(service: &Service, token: &str, builder: RequestBuilder) -> Value {
        let builder = builder.add_header("authorization", format!("Bearer {token}"), true);
        let router = service.router();
        let mut response = tokio::spawn(async move { builder.send(router).await })
            .await
            .unwrap();
        let value: Value = response.take_json().await.unwrap();
        assert_eq!(response.status_code, Some(StatusCode::OK), "{value}");
        value
    }

    async fn get(service: &Service, token: &str, path: &str) -> Value {
        request(
            service,
            token,
            TestClient::get(format!("http://localhost/{path}")),
        )
        .await
    }

    /// Sends a message to `room` and returns its event ID.
    async fn send(service: &Service, token: &str, room: &str, txn: &str, content: Value) -> String {
        let sent = request(
            service,
            token,
            TestClient::put(format!(
                "http://localhost/rooms/{room}/send/m.room.message/{txn}"
            ))
            .json(&content),
        )
        .await;
        sent["event_id"].as_str().unwrap().to_owned()
    }

    async fn create_room(service: &Service, token: &str) -> String {
        let created = request(
            service,
            token,
            TestClient::post("http://localhost/createRoom")
                .json(&json!({"room_version":"11","preset":"public_chat"})),
        )
        .await;
        created["room_id"].as_str().unwrap().to_owned()
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
    fn database_thread_summary_holds_a_reply_that_its_reader_may_see() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(run());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn run() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"threads.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let mut users = Vec::new();
        for (name, token) in [("summarya", "summary-a"), ("summaryb", "summary-b")] {
            let user =
                crate::core::UserId::parse(format!("@{name}:{}", crate::config::server_name()))
                    .unwrap();
            crate::data::user::create_user(&crate::data::user::NewDbUser {
                id: user.clone(),
                ty: None,
                is_admin: false,
                is_guest: false,
                is_local: true,
                localpart: name.into(),
                server_name: user.server_name().to_owned(),
                appservice_id: None,
                created_at: crate::core::UnixMillis::now(),
            })
            .await
            .unwrap();
            crate::data::user::device::create_device(
                &user,
                "SUMMARY_TEST".into(),
                token,
                None,
                None,
            )
            .await
            .unwrap();
            users.push(user);
        }
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(
                    Router::with_path("createRoom").post(crate::routing::client::room::create_room),
                )
                .push(crate::routing::client::room::authed_router()),
        );
        let (member, leaver) = ("summary-a", "summary-b");
        let room = &create_room(&service, member).await;
        request(
            &service,
            leaver,
            TestClient::post(format!("http://localhost/rooms/{room}/join")).json(&json!({})),
        )
        .await;
        let root = &send(
            &service,
            member,
            room,
            "summary-root",
            json!({"msgtype":"m.text","body":"first event"}),
        )
        .await;
        let reply = |body: &str| {
            json!({"msgtype":"m.text","body":body,"m.relates_to":{"rel_type":"m.thread",
                "event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}})
        };
        let summary = |event: &Value| event["unsigned"]["m.relations"]["m.thread"].clone();
        // The thread's first event, as one of the events that a response lists under `key`.
        let listed = |response: &Value, key: &str| {
            response[key]
                .as_array()
                .and_then(|events| events.iter().find(|event| event["event_id"] == *root))
                .unwrap_or_else(|| panic!("the first event is not in {key}: {response}"))
                .clone()
        };
        let alone = &format!("rooms/{room}/event/{root}");
        let threads = &format!("rooms/{room}/threads?include=all");
        let messages = &format!("rooms/{room}/messages?dir=b&limit=50");

        // While both are in the room, both are shown the reply.
        let seen = &send(&service, leaver, room, "summary-seen", reply("seen reply")).await;
        for token in [member, leaver] {
            let event = get(&service, token, alone).await;
            assert_eq!(
                summary(&event)["latest_event"]["event_id"],
                *seen,
                "{event}"
            );
            assert_eq!(summary(&event)["count"], 1, "{event}");
        }

        // A reader who ignores the reply's sender is shown no summary.
        crate::data::user::set_ignored_users(&users[0], std::slice::from_ref(&users[1]))
            .await
            .unwrap();
        let event = get(&service, member, alone).await;
        assert!(summary(&event).is_null(), "{event}");
        crate::data::user::set_ignored_users(&users[0], &[])
            .await
            .unwrap();
        let event = get(&service, member, alone).await;
        assert_eq!(
            summary(&event)["latest_event"]["event_id"],
            *seen,
            "{event}"
        );

        // One of them is removed, and the thread goes on.
        request(
            &service,
            member,
            TestClient::post(format!("http://localhost/rooms/{room}/kick"))
                .json(&json!({"user_id": users[1]})),
        )
        .await;
        let unseen = &send(
            &service,
            member,
            room,
            "summary-unseen",
            reply("unseen reply"),
        )
        .await;
        let around = &format!("rooms/{room}/context/{seen}?limit=10");

        // The member is shown the new reply, wherever the first event is served.
        let responses = [
            get(&service, member, alone).await,
            listed(&get(&service, member, threads).await, "chunk"),
            listed(&get(&service, member, around).await, "events_before"),
            listed(&get(&service, member, messages).await, "chunk"),
        ];
        for event in &responses {
            assert_eq!(
                summary(event)["latest_event"]["event_id"],
                *unseen,
                "{event}"
            );
            assert_eq!(summary(event)["count"], 2, "{event}");
        }

        // The removed user may still read the first event, and is shown nothing of a reply
        // that was sent after the removal.
        let responses = [
            (get(&service, leaver, alone).await, None),
            (get(&service, leaver, threads).await, Some("chunk")),
            (get(&service, leaver, around).await, Some("events_before")),
            (get(&service, leaver, messages).await, Some("chunk")),
        ];
        for (response, key) in &responses {
            let event = key.map_or_else(|| response.clone(), |key| listed(response, key));
            assert!(summary(&event).is_null(), "{event}");
            let response = response.to_string();
            assert!(!response.contains("unseen reply"), "{response}");
            assert!(!response.contains(unseen.as_str()), "{response}");
        }

        // A redacted reply is shown as it is now, not as it was sent.
        request(
            &service,
            member,
            TestClient::put(format!(
                "http://localhost/rooms/{room}/redact/{unseen}/summary-redact"
            ))
            .json(&json!({})),
        )
        .await;
        let event = get(&service, member, alone).await;
        assert_eq!(
            summary(&event)["latest_event"]["event_id"],
            *unseen,
            "{event}"
        );
        assert!(!event.to_string().contains("unseen reply"), "{event}");

        // An event of another room is no reply in this thread.
        let elsewhere = &create_room(&service, member).await;
        send(
            &service,
            member,
            elsewhere,
            "summary-elsewhere",
            reply("from another room"),
        )
        .await;
        let event = get(&service, member, alone).await;
        assert_eq!(
            summary(&event)["latest_event"]["event_id"],
            *unseen,
            "{event}"
        );
        assert_eq!(summary(&event)["count"], 2, "{event}");
    }
}
