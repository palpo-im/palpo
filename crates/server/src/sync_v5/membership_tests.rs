//! What a user is sent of a room follows from their membership of it. A member is sent the
//! room. A user who holds an invitation or a knock is sent its stripped state. A user who left
//! the room or was removed from it is sent their own membership. A user who has no part in the
//! room is sent nothing of it, whether a list or a subscription names the room.

use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;

const SYNC: &str = "http://localhost/_matrix/client/unstable/org.matrix.simplified_msc3575/sync";
/// What a client asks of a room it wants to know everything about.
const ROOM_STATE: &[[&str; 2]] = &[
    ["m.room.create", ""],
    ["m.room.name", ""],
    ["m.room.topic", ""],
    ["m.room.power_levels", ""],
    ["m.room.member", "*"],
];
/// Marks what a room's owner writes after a member has gone.
const LATER: &str = "written-later";

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
            "server_name": "membership.example", "db": {"url": "unused-test-config"},
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

async fn request(service: &Service, token: &str, request: RequestBuilder, body: Value) -> Value {
    let mut response = send(
        request
            .add_header("Authorization", format!("Bearer {token}"), true)
            .json(&body),
        service,
    )
    .await;
    let value = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{value}");
    value
}

async fn post(service: &Service, token: &str, path: &str, body: Value) -> Value {
    let url = format!("http://localhost/_matrix/client/v3/{path}");
    request(service, token, TestClient::post(url), body).await
}

async fn put(service: &Service, token: &str, path: &str, body: Value) -> Value {
    let url = format!("http://localhost/_matrix/client/v3/{path}");
    request(service, token, TestClient::put(url), body).await
}

/// A named room with a topic, created by `owner`'s user, with `member` in it. The owner has
/// sent a message, the member has read it, and the member is typing. `join_rule` is who may
/// enter the room.
async fn room(
    service: &Service,
    owner: &str,
    (member, member_id): (&str, &UserId),
    join_rule: &str,
) -> OwnedRoomId {
    let created = post(
        service,
        owner,
        "createRoom",
        json!({
            "room_version": "11",
            "preset": "private_chat",
            "name": "Members only",
            "topic": "Not for outsiders",
            "initial_state": [{
                "type": "m.room.join_rules",
                "state_key": "",
                "content": {"join_rule": join_rule},
            }],
        }),
    )
    .await;
    let room_id = OwnedRoomId::try_from(created["room_id"].as_str().unwrap()).unwrap();
    invite(service, owner, &room_id, member_id).await;
    join(service, member, &room_id).await;
    let sent = post(
        service,
        owner,
        &format!("rooms/{room_id}/send/m.room.message"),
        json!({"msgtype": "m.text", "body": "for members"}),
    )
    .await;
    let event_id = sent["event_id"].as_str().unwrap();
    post(
        service,
        member,
        &format!("rooms/{room_id}/receipt/m.read/{event_id}"),
        json!({}),
    )
    .await;
    put(
        service,
        member,
        &format!("rooms/{room_id}/typing/{member_id}"),
        json!({"typing": true, "timeout": 30000}),
    )
    .await;
    room_id
}

async fn invite(service: &Service, token: &str, room_id: &RoomId, invitee: &UserId) {
    post(
        service,
        token,
        &format!("rooms/{room_id}/invite"),
        json!({"user_id": invitee}),
    )
    .await;
}

async fn join(service: &Service, token: &str, room_id: &RoomId) {
    post(service, token, &format!("rooms/{room_id}/join"), json!({})).await;
}

/// The owner renames the room and sends a message, both marked with `name`.
async fn write_later(service: &Service, owner: &str, room_id: &RoomId, name: &str) {
    put(
        service,
        owner,
        &format!("rooms/{room_id}/state/m.room.name"),
        json!({"name": format!("{name} {LATER}")}),
    )
    .await;
    post(
        service,
        owner,
        &format!("rooms/{room_id}/send/m.room.message"),
        json!({"msgtype": "m.text", "body": format!("{name} {LATER}")}),
    )
    .await;
}

/// One request of the connection `conn_id`, which asks for everything of the rooms that
/// `selection` names: their state with every member, their timeline, their heroes, their
/// receipts and who is typing.
async fn sync(
    service: &Service,
    token: &str,
    conn_id: &str,
    pos: Option<&str>,
    selection: Value,
) -> Value {
    let url = match pos {
        Some(pos) => format!("{SYNC}?timeout=0&pos={pos}"),
        None => format!("{SYNC}?timeout=0"),
    };
    let mut body = json!({
        "conn_id": conn_id,
        "extensions": {"receipts": {"enabled": true}, "typing": {"enabled": true}},
    });
    body.as_object_mut()
        .unwrap()
        .extend(selection.as_object().unwrap().clone());
    request(service, token, TestClient::post(url), body).await
}

/// A request of a room list connection, as a client keeps one for all of the user's rooms.
async fn list(service: &Service, token: &str, pos: Option<&str>) -> Value {
    let selection = json!({"lists": {"all_rooms": {
        "ranges": [[0, 20]],
        "timeline_limit": 10,
        "required_state": ROOM_STATE,
        "include_heroes": true,
    }}});
    sync(service, token, "room-list", pos, selection).await
}

/// A request of a connection that subscribes to `room_id`, as a client does for a room it
/// shows.
async fn subscription(
    service: &Service,
    token: &str,
    conn_id: &str,
    pos: Option<&str>,
    room_id: &RoomId,
) -> Value {
    let selection = json!({"room_subscriptions": {room_id.as_str(): {
        "timeline_limit": 10,
        "required_state": ROOM_STATE,
        "include_heroes": true,
    }}});
    sync(service, token, conn_id, pos, selection).await
}

fn pos(response: &Value) -> &str {
    response["pos"].as_str().unwrap()
}

fn is_empty(value: &Value) -> bool {
    value.as_array().is_none_or(|values| values.is_empty())
}

/// Whether `response` holds receipts or typing users of `room_id`.
fn has_ephemeral(response: &Value, room_id: &RoomId) -> bool {
    ["receipts", "typing"].iter().any(|extension| {
        response["extensions"][extension]["rooms"]
            .get(room_id.as_str())
            .is_some()
    })
}

/// The membership that `user_id`'s own member event has in the room state `room` holds.
fn own_membership<'a>(room: &'a Value, user_id: &UserId) -> Option<&'a str> {
    let own = room["required_state"]
        .as_array()?
        .iter()
        .find(|event| event["type"] == "m.room.member" && event["state_key"] == user_id.as_str())?;
    own["content"]["membership"].as_str()
}

/// Asserts that `response` serves `room_id` to `user_id` as to a member: with its name, its
/// topic, its members and its timeline.
fn assert_served_in_full(response: &Value, room_id: &RoomId, user_id: &UserId) {
    let room = &response["rooms"][room_id.as_str()];
    assert!(room["invite_state"].is_null(), "{response}");
    assert_eq!(own_membership(room, user_id), Some("join"), "{response}");
    let state = room["required_state"].as_array().unwrap();
    for ty in ["m.room.name", "m.room.topic"] {
        assert!(
            state.iter().any(|event| event["type"] == ty),
            "{ty} is missing: {response}"
        );
    }
    let members = state
        .iter()
        .filter(|event| event["type"] == "m.room.member")
        .count();
    assert!(members > 1, "{response}");
    assert!(!is_empty(&room["timeline"]), "{response}");
    assert!(room["joined_count"].as_i64() > Some(1), "{response}");
}

/// Asserts that `response` says nothing about `room_id`.
fn assert_not_served(response: &Value, room_id: &RoomId) {
    assert!(
        response["rooms"].get(room_id.as_str()).is_none(),
        "{response}"
    );
    assert!(!has_ephemeral(response, room_id), "{response}");
}

/// Asserts that `response` serves `room_id` as stripped state that holds `user_id`'s own
/// `membership`, and nothing of what the room's members are sent.
fn assert_stripped(response: &Value, room_id: &RoomId, user_id: &UserId, membership: &str) {
    let room = &response["rooms"][room_id.as_str()];
    let own = room["invite_state"]
        .as_array()
        .and_then(|events| {
            events.iter().find(|event| {
                event["type"] == "m.room.member" && event["state_key"] == user_id.as_str()
            })
        })
        .unwrap_or_else(|| panic!("no stripped state with the user's own membership: {response}"));
    assert_eq!(own["content"]["membership"], membership, "{response}");
    assert!(is_empty(&room["required_state"]), "{response}");
    assert!(is_empty(&room["timeline"]), "{response}");
    assert!(!has_ephemeral(response, room_id), "{response}");
}

/// Asserts that `response` tells `user_id`, who has left `room_id`, no more of the room than
/// their own membership and the timeline of their time in the room: nothing of `unseen`, which
/// marks what the owner wrote after it.
fn assert_left(response: &Value, room_id: &RoomId, user_id: &UserId, unseen: &str) {
    let room = &response["rooms"][room_id.as_str()];
    assert!(room["invite_state"].is_null(), "{response}");
    for event in room["required_state"].as_array().into_iter().flatten() {
        assert_eq!(event["type"], "m.room.member", "{response}");
        assert_eq!(event["state_key"], user_id.as_str(), "{response}");
    }
    for field in ["name", "avatar", "heroes", "joined_count", "invited_count"] {
        assert!(room[field].is_null(), "{field}: {response}");
    }
    assert!(!has_ephemeral(response, room_id), "{response}");
    assert!(!response.to_string().contains(unseen), "{response}");
}

const OWNER: &str = "sliding-membership-owner";
const MEMBER: &str = "sliding-membership-member";
const STRANGER: &str = "sliding-membership-stranger";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_serves_a_subscription_to_a_user_of_the_room_only() {
    run_database_test("sliding-sync-stranger", subscription_of_a_stranger);
}

async fn subscription_of_a_stranger() {
    configure();
    create_user("membership-owner", OWNER).await;
    let member = create_user("membership-member", MEMBER).await;
    let stranger = create_user("membership-stranger", STRANGER).await;
    let service = Service::new(crate::routing::root());
    let room_id = room(&service, OWNER, (MEMBER, &member), "invite").await;

    let served = subscription(&service, MEMBER, "room", None, &room_id).await;
    assert_served_in_full(&served, &room_id, &member);

    // A user who knows the room's ID and has never had a part in the room.
    let refused = subscription(&service, STRANGER, "room", None, &room_id).await;
    assert_not_served(&refused, &room_id);

    // Once the user is in the room, the same connection is sent it as a new room.
    invite(&service, OWNER, &room_id, &stranger).await;
    join(&service, STRANGER, &room_id).await;
    let joined = subscription(&service, STRANGER, "room", Some(pos(&refused)), &room_id).await;
    assert_served_in_full(&joined, &room_id, &stranger);
    assert_eq!(
        joined["rooms"][room_id.as_str()]["initial"],
        json!(true),
        "{joined}"
    );
}

const FORMER_OWNER: &str = "sliding-membership-former-owner";
const FORMER_MEMBER: &str = "sliding-membership-former-member";
const FORMER: &str = "sliding-membership-former";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_sends_a_user_who_left_their_own_membership_only() {
    run_database_test("sliding-sync-former", former_member);
}

async fn former_member() {
    configure();
    create_user("membership-former-owner", FORMER_OWNER).await;
    let member = create_user("membership-former-member", FORMER_MEMBER).await;
    let former = create_user("membership-former", FORMER).await;
    let service = Service::new(crate::routing::root());
    let room_id = room(&service, FORMER_OWNER, (FORMER_MEMBER, &member), "invite").await;
    invite(&service, FORMER_OWNER, &room_id, &former).await;
    join(&service, FORMER, &room_id).await;
    let joined = subscription(&service, FORMER, "room", None, &room_id).await;
    assert_served_in_full(&joined, &room_id, &former);

    post(
        &service,
        FORMER_OWNER,
        &format!("rooms/{room_id}/kick"),
        json!({"user_id": former}),
    )
    .await;
    write_later(&service, FORMER_OWNER, &room_id, "First").await;

    // The connection that holds the room as joined is told that the user is out of it ...
    let removed = subscription(&service, FORMER, "room", Some(pos(&joined)), &room_id).await;
    assert_left(&removed, &room_id, &former, LATER);
    let room = &removed["rooms"][room_id.as_str()];
    assert_eq!(own_membership(room, &former), Some("leave"), "{removed}");
    // ... and hears no more of the room.
    write_later(&service, FORMER_OWNER, &room_id, "Second").await;
    let later = subscription(&service, FORMER, "room", Some(pos(&removed)), &room_id).await;
    assert_left(&later, &room_id, &former, LATER);
    assert!(
        is_empty(&later["rooms"][room_id.as_str()]["required_state"]),
        "{later}"
    );

    // A new connection is sent no more either, by a subscription or by a list. A client that
    // was closed during the removal still holds the room as joined and subscribes to it, as
    // matrix-rust-sdk does: with the members of the timeline and the user's own membership.
    let selection = json!({"room_subscriptions": {room_id.as_str(): {
        "timeline_limit": 20,
        "required_state": [
            ["m.room.name", ""],
            ["m.room.member", "$LAZY"],
            ["m.room.member", "$ME"],
        ],
    }}});
    let fresh = sync(&service, FORMER, "fresh", None, selection).await;
    assert_left(&fresh, &room_id, &former, LATER);
    let room = &fresh["rooms"][room_id.as_str()];
    assert_eq!(own_membership(room, &former), Some("leave"), "{fresh}");
    assert_not_served(&list(&service, FORMER, None).await, &room_id);

    // Back in the room, the user is sent what the room has become.
    invite(&service, FORMER_OWNER, &room_id, &former).await;
    join(&service, FORMER, &room_id).await;
    let back = subscription(&service, FORMER, "room", Some(pos(&later)), &room_id).await;
    let room = &back["rooms"][room_id.as_str()];
    assert_eq!(own_membership(room, &former), Some("join"), "{back}");
    let name = room["required_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "m.room.name")
        .unwrap_or_else(|| panic!("no name: {back}"));
    assert_eq!(name["content"]["name"], format!("Second {LATER}"), "{back}");
    assert!(!is_empty(&room["timeline"]), "{back}");

    // Banned from the room, the user is told so and no more.
    post(
        &service,
        FORMER_OWNER,
        &format!("rooms/{room_id}/ban"),
        json!({"user_id": former}),
    )
    .await;
    write_later(&service, FORMER_OWNER, &room_id, "Third").await;
    let banned = subscription(&service, FORMER, "room", Some(pos(&back)), &room_id).await;
    assert_left(&banned, &room_id, &former, "Third");
    let room = &banned["rooms"][room_id.as_str()];
    assert_eq!(own_membership(room, &former), Some("ban"), "{banned}");
}

const KNOCK_OWNER: &str = "sliding-membership-knock-owner";
const KNOCK_MEMBER: &str = "sliding-membership-knock-member";
const KNOCKER: &str = "sliding-membership-knocker";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_sends_a_knocked_room_as_stripped_state() {
    run_database_test("sliding-sync-knock", knocked_room);
}

async fn knocked_room() {
    configure();
    create_user("membership-knock-owner", KNOCK_OWNER).await;
    let member = create_user("membership-knock-member", KNOCK_MEMBER).await;
    let knocker = create_user("membership-knocker", KNOCKER).await;
    let service = Service::new(crate::routing::root());
    let room_id = room(&service, KNOCK_OWNER, (KNOCK_MEMBER, &member), "knock").await;
    // The user's client is connected before the user knocks.
    let before = list(&service, KNOCKER, None).await;
    assert_not_served(&before, &room_id);
    post(&service, KNOCKER, &format!("knock/{room_id}"), json!({})).await;

    // The room has not let the user in: neither a list nor a subscription shows its members.
    let knocked = list(&service, KNOCKER, Some(pos(&before))).await;
    let subscribed = subscription(&service, KNOCKER, "room", None, &room_id).await;
    for response in [&knocked, &subscribed] {
        assert_stripped(response, &room_id, &knocker, "knock");
        let room = &response["rooms"][room_id.as_str()];
        for field in ["heroes", "joined_count", "invited_count"] {
            assert!(room[field].is_null(), "{field}: {response}");
        }
    }
    // A response may never reach the client, which then asks again from where it was: the
    // knock goes out until the client returns with the token of a response that carried it.
    let again = list(&service, KNOCKER, Some(pos(&before))).await;
    assert_stripped(&again, &room_id, &knocker, "knock");
    // From then on a knock that stands is no news, whatever happens in the room.
    write_later(&service, KNOCK_OWNER, &room_id, "First").await;
    let waiting = list(&service, KNOCKER, Some(pos(&again))).await;
    assert_not_served(&waiting, &room_id);
    assert!(!waiting.to_string().contains(LATER), "{waiting}");

    // The owner lets the user in: the room goes out as an invitation ...
    invite(&service, KNOCK_OWNER, &room_id, &knocker).await;
    let invited = subscription(&service, KNOCKER, "room", Some(pos(&subscribed)), &room_id).await;
    assert_stripped(&invited, &room_id, &knocker, "invite");
    // ... and as a new room once the user has joined it, to the connection that last held
    // the knock and to the one that held the invitation.
    join(&service, KNOCKER, &room_id).await;
    for joined in [
        list(&service, KNOCKER, Some(pos(&waiting))).await,
        subscription(&service, KNOCKER, "room", Some(pos(&invited)), &room_id).await,
    ] {
        assert_served_in_full(&joined, &room_id, &knocker);
        assert_eq!(
            joined["rooms"][room_id.as_str()]["initial"],
            json!(true),
            "{joined}"
        );
    }
}

const INVITER: &str = "sliding-membership-inviter";
const INVITER_MEMBER: &str = "sliding-membership-inviter-member";
const INVITEE: &str = "sliding-membership-invitee";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_sends_an_invited_user_no_receipts_and_no_typing() {
    run_database_test("sliding-sync-invited", invited_room);
}

async fn invited_room() {
    configure();
    create_user("membership-inviter", INVITER).await;
    let member = create_user("membership-inviter-member", INVITER_MEMBER).await;
    let invitee = create_user("membership-invitee", INVITEE).await;
    let service = Service::new(crate::routing::root());
    let room_id = room(&service, INVITER, (INVITER_MEMBER, &member), "invite").await;
    invite(&service, INVITER, &room_id, &invitee).await;

    // The room's members are sent who has read what and who is typing ...
    let members = list(&service, INVITER, None).await;
    for extension in ["receipts", "typing"] {
        assert!(
            members["extensions"][extension]["rooms"]
                .get(room_id.as_str())
                .is_some(),
            "{extension}: {members}"
        );
    }
    // ... and a user the room has invited is not.
    for response in [
        list(&service, INVITEE, None).await,
        subscription(&service, INVITEE, "room", None, &room_id).await,
    ] {
        assert_stripped(&response, &room_id, &invitee, "invite");
    }
}

const DECLINED_INVITER: &str = "sliding-membership-declined-inviter";
const DECLINED_MEMBER: &str = "sliding-membership-declined-member";
const DECLINER: &str = "sliding-membership-decliner";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_sliding_sync_does_not_send_an_invitation_declined_during_the_request() {
    run_database_test(
        "sliding-sync-late-decline",
        invitation_declined_during_the_request,
    );
}

async fn invitation_declined_during_the_request() {
    configure();
    create_user("membership-declined-inviter", DECLINED_INVITER).await;
    let member = create_user("membership-declined-member", DECLINED_MEMBER).await;
    let invitee = create_user("membership-decliner", DECLINER).await;
    let device = OwnedDeviceId::from("D");
    let service = Service::new(crate::routing::root());
    let room_id = room(
        &service,
        DECLINED_INVITER,
        (DECLINED_MEMBER, &member),
        "invite",
    )
    .await;
    invite(&service, DECLINED_INVITER, &room_id, &invitee).await;

    // A request reads the invitations when it begins ...
    let since_sn = data::curr_sn().await.unwrap();
    let invitations = crate::membership::invited_rooms_for_sync(&invitee, 0, &device, since_sn)
        .await
        .unwrap();
    assert!(invitations.rooms.contains_key(&room_id));
    // ... and the user declines the invitation before the request builds the room.
    post(
        &service,
        DECLINER,
        &format!("rooms/{room_id}/leave"),
        json!({}),
    )
    .await;

    let req_body: SyncEventsReqBody =
        serde_json::from_value(json!({"conn_id": "late-decline"})).unwrap();
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
    // The room goes out as one the user has left, not as an invitation.
    let response = json!({"rooms": rooms});
    assert_left(&response, &room_id, &invitee, LATER);
    let room = &response["rooms"][room_id.as_str()];
    assert_eq!(own_membership(room, &invitee), Some("leave"), "{response}");
}
