//! A room the client holds as an invitation goes out as a new joined room once the user
//! joins it, with the user's own join event in its state, and never with invite state after
//! the join. Invited users get the invitation's stripped state only.

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const SYNC: &str = "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync";
/// Room state with the user's own membership, as matrix-rust-sdk's room list requests it.
const ROOM_STATE: &[[&str; 2]] = &[
    ["m.room.create", ""],
    ["m.room.encryption", ""],
    ["m.room.name", ""],
    ["m.room.member", "$ME"],
];

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
            "server_name": "invitation.example", "db": {"url": "unused-test-config"},
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

/// An encrypted, invitation-only room created by `token`'s user, who invites `invitee`.
async fn invite(service: &Service, token: &str, invitee: &UserId) -> OwnedRoomId {
    let created = client_api(
        service,
        token,
        "createRoom",
        json!({
            "room_version": "11",
            "preset": "private_chat",
            "name": "Invitation room",
            "initial_state": [{
                "type": "m.room.encryption",
                "state_key": "",
                "content": {"algorithm": "m.megolm.v1.aes-sha2"},
            }],
        }),
    )
    .await;
    let room_id = OwnedRoomId::try_from(created["room_id"].as_str().unwrap()).unwrap();
    client_api(
        service,
        token,
        &format!("rooms/{room_id}/invite"),
        json!({"user_id": invitee}),
    )
    .await;
    room_id
}

/// One request of a client's room list connection, as matrix-rust-sdk sends it.
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
            "conn_id": "room-list",
            "lists": {"all_rooms": {
                "ranges": [[0, 20]],
                "timeline_limit": 1,
                "required_state": ROOM_STATE,
            }},
        }),
    )
    .await
}

fn pos(response: &Value) -> &str {
    response["pos"].as_str().unwrap()
}

fn state_types(room: &Value) -> BTreeSet<&str> {
    room["required_state"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .map(|event| event["type"].as_str().unwrap())
                .collect()
        })
        .unwrap_or_default()
}

/// Asserts that `room`, as a response renders it, is a joined room with its full state,
/// including `user_id`'s own join event -- the only thing that tells matrix-rust-sdk that a
/// room it holds as an invitation was joined.
fn assert_joined_in_full(room: &Value, user_id: &UserId) {
    assert!(room["invite_state"].is_null(), "{room}");
    assert_eq!(
        state_types(room),
        BTreeSet::from([
            "m.room.create",
            "m.room.encryption",
            "m.room.member",
            "m.room.name"
        ]),
        "{room}"
    );
    let own = room["required_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "m.room.member" && event["state_key"] == user_id.as_str())
        .expect("the user's own membership");
    assert_eq!(own["content"]["membership"], "join", "{room}");
}

const INVITER: &str = "sliding-invitation-inviter";
const INVITEE: &str = "sliding-invitation-invitee";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_sends_an_accepted_invitation_as_a_new_room() {
    run_database_test("sliding-sync-invitation", accepted_invitation);
}

async fn accepted_invitation() {
    configure();
    create_user("invitation-inviter", INVITER).await;
    let invitee = create_user("invitation-invitee", INVITEE).await;
    let service = Service::new(crate::routing::root());

    let start = sync(&service, INVITEE, None).await;
    let room_id = invite(&service, INVITER, &invitee).await;

    // An invited user is sent the invitation's stripped state, not the room's state.
    let invited = sync(&service, INVITEE, Some(pos(&start))).await;
    let room = &invited["rooms"][room_id.as_str()];
    assert!(
        room["invite_state"]
            .as_array()
            .is_some_and(|state| !state.is_empty()),
        "{invited}"
    );
    assert!(state_types(room).is_empty(), "{invited}");

    client_api(
        &service,
        INVITEE,
        &format!("rooms/{room_id}/join"),
        json!({}),
    )
    .await;

    // The client holds the room as an invitation, so the room goes out as new.
    let joined = sync(&service, INVITEE, Some(pos(&invited))).await;
    let room = &joined["rooms"][room_id.as_str()];
    assert_joined_in_full(room, &invitee);
    assert_eq!(room["initial"], json!(true), "{joined}");

    // That response may never reach the client, which then asks again from where it was.
    let retry = sync(&service, INVITEE, Some(pos(&invited))).await;
    assert_joined_in_full(&retry["rooms"][room_id.as_str()], &invitee);

    // Once the client returns with that response's token, the room is an ordinary one.
    let next = sync(&service, INVITEE, Some(pos(&retry))).await;
    let room = &next["rooms"][room_id.as_str()];
    assert!(room["invite_state"].is_null(), "{next}");
    assert!(state_types(room).is_empty(), "{next}");
}

const LATE_INVITER: &str = "sliding-invitation-late-inviter";
const LATE_INVITEE: &str = "sliding-invitation-late-invitee";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_does_not_send_an_invitation_accepted_during_the_request() {
    run_database_test(
        "sliding-sync-late-join",
        invitation_accepted_during_the_request,
    );
}

async fn invitation_accepted_during_the_request() {
    configure();
    create_user("invitation-late-inviter", LATE_INVITER).await;
    let invitee = create_user("invitation-late-invitee", LATE_INVITEE).await;
    let device = OwnedDeviceId::from("D");
    let service = Service::new(crate::routing::root());
    let room_id = invite(&service, LATE_INVITER, &invitee).await;

    // A request reads the invitations when it begins ...
    let since_sn = data::curr_sn().await.unwrap();
    let invitations = crate::membership::invited_rooms_for_sync(&invitee, 0, &device, since_sn)
        .await
        .unwrap();
    assert!(invitations.rooms.contains_key(&room_id));
    // ... and the user accepts the invitation before the request builds the room.
    client_api(
        &service,
        LATE_INVITEE,
        &format!("rooms/{room_id}/join"),
        json!({}),
    )
    .await;

    let req_body: SyncEventsReqBody =
        serde_json::from_value(json!({"conn_id": "late-join"})).unwrap();
    let required_state = ROOM_STATE
        .iter()
        .map(|[ty, key]| (StateEventType::from(*ty), (*key).to_owned()))
        .collect();
    let mut response = SyncEventsResBody::new(String::new());
    let (rooms, ..) = process_rooms(
        SyncInfo {
            sender_id: &invitee,
            device_id: &device,
            since_sn,
            req_body: &req_body,
        },
        &invitations,
        &HashSet::new(),
        &BTreeSet::new(),
        &[(room_id.clone(), TodoRoom::new(required_state, 1, since_sn))].into(),
        &KnownRooms::new(),
        &mut response,
        #[cfg(feature = "unstable-msc4480")]
        &sticky::Offer::default(),
    )
    .await
    .unwrap();
    let room = serde_json::to_value(&rooms[&room_id]).unwrap();
    assert_joined_in_full(&room, &invitee);
    assert_eq!(room["initial"], json!(true), "{room}");
}
