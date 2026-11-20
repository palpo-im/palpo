//! A room's lazily loaded members include the targets of the membership events in its
//! timeline. A kick, ban or invitation by another user changes the target's membership, and
//! clients such as matrix-rust-sdk take a room's members from its required state only.

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const SYNC: &str = "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync";
/// The state a room's timeline needs, with the members it names loaded lazily.
const LAZY_MEMBERS: &[[&str; 2]] = &[["m.room.create", ""], ["m.room.member", "$LAZY"]];

/// Runs a database test on a thread with room for the server's deep futures.
fn run_database_test<F>(name: &str, test: fn() -> F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(test())
        })
        .unwrap()
        .join()
        .unwrap();
}

fn configure() {
    crate::test_database::init();
    crate::config::CONFIG.get_or_init(|| {
        serde_json::from_value(json!({
            "server_name": "lazy-members.example", "db": {"url": "unused-test-config"},
            "ip_range_denylist": [],
        }))
        .unwrap()
    });
}

/// A local user with device `D`, signed in with `token`.
async fn create_user(localpart: &str, token: &str) -> OwnedUserId {
    let user_id = UserId::parse(format!("@{localpart}:{}", crate::config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user_id.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: localpart.into(),
        server_name: user_id.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&user_id).await.unwrap();
    data::user::device::create_device(&user_id, &OwnedDeviceId::from("D"), token, None, None)
        .await
        .unwrap();
    user_id
}

async fn send(request: RequestBuilder, service: &Service) -> Response {
    let router = service.router();
    tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap()
}

async fn post(service: &Service, token: &str, url: String, body: Value) -> Value {
    let mut response = send(
        TestClient::post(url)
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&body),
        service,
    )
    .await;
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{value}");
    value
}

async fn client_api(service: &Service, token: &str, path: &str, body: Value) -> Value {
    post(
        service,
        token,
        format!("http://localhost/_matrix/client/v3/{path}"),
        body,
    )
    .await
}

/// One request of a client's connection for the room's timeline, with lazy members.
async fn sync(service: &Service, token: &str, pos: Option<&str>) -> Value {
    let url = match pos {
        Some(pos) => format!("{SYNC}?timeout=0&pos={pos}"),
        None => format!("{SYNC}?timeout=0"),
    };
    post(
        service,
        token,
        url,
        json!({
            "conn_id": "lazy-members",
            "lists": {"all_rooms": {
                "ranges": [[0, 20]],
                "timeline_limit": 1,
                "required_state": LAZY_MEMBERS,
            }},
        }),
    )
    .await
}

fn pos(response: &Value) -> &str {
    response["pos"].as_str().unwrap()
}

/// `user_id`'s membership as the response's required state for `room_id` carries it.
fn membership<'a>(response: &'a Value, room_id: &RoomId, user_id: &UserId) -> Option<&'a str> {
    response["rooms"][room_id.as_str()]["required_state"]
        .as_array()?
        .iter()
        .find(|event| event["type"] == "m.room.member" && event["state_key"] == user_id.as_str())
        .and_then(|event| event["content"]["membership"].as_str())
}

const OWNER: &str = "sliding-lazy-owner";
const WATCHER: &str = "sliding-lazy-watcher";
const MEMBER: &str = "sliding-lazy-member";
const GUEST: &str = "sliding-lazy-guest";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_sends_the_targets_of_membership_changes_as_lazy_members() {
    run_database_test("sliding-sync-lazy-members", membership_change_targets);
}

/// The owner removes a member and invites a guest. The watcher's client learns both changes
/// only through the targets' membership events in the room's required state.
async fn membership_change_targets() {
    configure();
    create_user("lazy-owner", OWNER).await;
    let watcher = create_user("lazy-watcher", WATCHER).await;
    let member = create_user("lazy-member", MEMBER).await;
    let guest = create_user("lazy-guest", GUEST).await;
    let service = Service::new(crate::routing::root());
    let created = client_api(
        &service,
        OWNER,
        "createRoom",
        json!({"room_version": "11", "preset": "private_chat", "name": "Lazy members room"}),
    )
    .await;
    let room_id = OwnedRoomId::try_from(created["room_id"].as_str().unwrap()).unwrap();
    for (user_id, token) in [(&watcher, WATCHER), (&member, MEMBER)] {
        client_api(
            &service,
            OWNER,
            &format!("rooms/{room_id}/invite"),
            json!({"user_id": user_id}),
        )
        .await;
        client_api(&service, token, &format!("rooms/{room_id}/join"), json!({})).await;
    }
    let start = sync(&service, WATCHER, None).await;
    assert_eq!(
        membership(&start, &room_id, &member),
        Some("join"),
        "{start}"
    );

    client_api(
        &service,
        OWNER,
        &format!("rooms/{room_id}/kick"),
        json!({"user_id": member}),
    )
    .await;
    let kicked = sync(&service, WATCHER, Some(pos(&start))).await;
    assert_eq!(
        membership(&kicked, &room_id, &member),
        Some("leave"),
        "{kicked}"
    );

    client_api(
        &service,
        OWNER,
        &format!("rooms/{room_id}/invite"),
        json!({"user_id": guest}),
    )
    .await;
    let invited = sync(&service, WATCHER, Some(pos(&kicked))).await;
    assert_eq!(
        membership(&invited, &room_id, &guest),
        Some("invite"),
        "{invited}"
    );
}
