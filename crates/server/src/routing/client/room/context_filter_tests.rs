//! Exercise context filtering through the authenticated production router.
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedUserId, UnixMillis, UserId};
use crate::{config, data};

fn events<'a>(body: &'a Value, section: &str) -> &'a [Value] {
    match body.get(section) {
        None => &[], // Empty context sections are omitted by the response model.
        Some(Value::Array(events)) => events,
        Some(other) => panic!("invalid {section} section: {other}"),
    }
}

async fn ok(service: &Service, token: &str, request: RequestBuilder) -> Value {
    let router = service.router();
    let request = request.add_header("Authorization", format!("Bearer {token}"), true);
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn account(localpart: &str) -> OwnedUserId {
    let user = UserId::parse(format!("@{localpart}:{}", config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: localpart.into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user).await.unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), localpart, None, None)
        .await
        .unwrap();
    user
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_context_filters_timeline_and_state_without_filtering_anchor() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(context_filters());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn context_filters() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"context-filter.example", "db":{"url":"unused"},
                "auto_join_rooms":[], "rc_message":{"per_second":10000.0,"burst":10000}
            }))
            .unwrap(),
        )
        .unwrap();
    let alice = account("alice").await;
    let bob = account("bob").await;
    let service = Service::new(crate::routing::root());
    let created = ok(
        &service,
        "alice",
        TestClient::post("http://localhost/_matrix/client/v3/createRoom").json(&json!({
            "room_version":"11", "preset":"public_chat",
            "initial_state":[
                {"type":"org.palpo.context.link", "state_key":"", "content":{"url":"https://example.org"}},
                {"type":"org.palpo.context.hidden", "state_key":"", "content":{"value":"excluded"}},
                {"type":"m.room.topic", "state_key":"", "content":{"topic":"plain state"}}
            ]
        })),
    )
    .await;
    let room = created["room_id"].as_str().unwrap();
    ok(
        &service,
        "bob",
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/join"
        ))
        .json(&json!({})),
    )
    .await;
    let mut anchor = String::new();
    for side in ["before", "after"] {
        for (index, (token, has_url)) in [("alice", false), ("bob", true), ("alice", true)]
            .into_iter()
            .enumerate()
        {
            let content = if has_url {
                json!({"msgtype":"m.image", "body":"image", "url":"mxc://context-filter.example/image"})
            } else {
                json!({"msgtype":"m.text", "body":"text"})
            };
            ok(
                &service,
                token,
                TestClient::put(format!(
                    "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/{side}-{index}"
                ))
                .json(&content),
            )
            .await;
        }
        if side == "before" {
            anchor = ok(
                &service,
                "bob",
                TestClient::put(format!(
                    "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/anchor"
                ))
                .json(&json!({"msgtype":"m.text", "body":"anchor is always returned"})),
            )
            .await["event_id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }

    for (filter, before_count, after_count) in [
        (json!({"types":["m.room.message"]}), 3, 3),
        (
            json!({"types":["m.room.message"],"senders":[alice,bob],"not_senders":[bob]}),
            2,
            2,
        ),
        (json!({"contains_url":true}), 3, 2),
        (
            json!({"types":["m.room.message"],"contains_url":false}),
            1,
            1,
        ),
        (
            json!({"types":["org.palpo.context.*"],"not_types":["org.palpo.context.hidden"]}),
            1,
            0,
        ),
        (json!({"types":[]}), 0, 0),
    ] {
        let mut url = url::Url::parse(&format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/context/{anchor}"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("limit", "100")
            .append_pair("filter", &filter.to_string());
        let response = ok(&service, "alice", TestClient::get(url.as_str())).await;
        assert_eq!(
            response["event"]["event_id"], anchor,
            "{filter}: {response}"
        );
        assert_eq!(
            events(&response, "events_before").len(),
            before_count,
            "{filter}: {response}"
        );
        assert_eq!(
            events(&response, "events_after").len(),
            after_count,
            "{filter}: {response}"
        );
        for section in ["events_before", "events_after", "state"] {
            for event in events(&response, section) {
                if section != "state" {
                    assert_ne!(
                        event["event_id"], anchor,
                        "anchor repeated in {section}: {response}"
                    );
                }
                if filter["not_senders"].is_array() {
                    assert_ne!(event["sender"], bob.as_str(), "{section}: {response}");
                }
                if filter["contains_url"] == true {
                    assert!(
                        event["content"].get("url").is_some(),
                        "{section}: {response}"
                    );
                } else if filter["contains_url"] == false {
                    assert!(
                        event["content"].get("url").is_none(),
                        "{section}: {response}"
                    );
                }
                assert!(
                    filter["not_types"]
                        .as_array()
                        .is_none_or(|types| types.iter().all(|ty| ty != &event["type"])),
                    "{section}: {response}"
                );
                if let Some(types) = filter["types"].as_array() {
                    assert!(
                        types
                            .iter()
                            .any(|ty| crate::event::search::event_type_matches(
                                ty.as_str().unwrap(),
                                event["type"].as_str().unwrap()
                            )),
                        "{section}: {response}"
                    );
                }
            }
        }
        if filter["types"] == json!(["m.room.message"]) || filter["types"] == json!([]) {
            assert!(events(&response, "state").is_empty(), "{response}");
        } else if filter["contains_url"] == true
            || filter["types"] == json!(["org.palpo.context.*"])
        {
            assert_eq!(events(&response, "state").len(), 1, "{response}");
            assert_eq!(response["state"][0]["type"], "org.palpo.context.link");
        }
    }
}
