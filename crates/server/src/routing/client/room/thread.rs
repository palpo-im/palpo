use salvo::prelude::*;

use crate::core::client::room::{ThreadsReqArgs, ThreadsResBody};
use crate::room::state;
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

    let (events, next_batch) = crate::room::thread::get_threads(
        &args.room_id,
        authed.user_id(),
        &args.include,
        limit,
        args.from.as_deref(),
    )
    .await?;

    let mut threads = Vec::new();
    for (sn, pdu) in events.into_iter() {
        if state::user_can_see_event(authed.user_id(), &pdu.event_id)
            .await
            .unwrap_or(false)
        {
            threads.push((sn, pdu));
        }
    }

    json_ok(ThreadsResBody {
        chunk: threads
            .into_iter()
            .map(|(_, pdu)| pdu.to_room_event_for(authed.user_id(), authed.device_id()))
            .collect(),
        next_batch,
    })
}

#[cfg(test)]
mod participation_tests {
    use super::*;
    use salvo::test::{RequestBuilder, ResponseExt, TestClient};
    use serde_json::{Value, json};

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
