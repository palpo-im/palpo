//! A room the client holds as an invitation goes out as a new joined room once the user
//! joins it, with the user's own join event in its state, and never with invite state after
//! the join. Invited users get the invitation's stripped state only.
//!
//! An invitation names the room's other members when the server is configured to, up to
//! `invite_state_member_limit` of them.

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

/// The `m.room.member` events of stripped state: whom each is about, and their membership.
fn members(state: &Value) -> Vec<(String, String)> {
    state
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "m.room.member")
        .map(|event| {
            // A stripped event is its type, state key, sender and content.
            assert_eq!(event.as_object().unwrap().len(), 4, "{event}");
            (
                event["state_key"].as_str().unwrap().to_owned(),
                event["content"]["membership"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn membership(user_id: &UserId, membership: &str) -> (String, String) {
    (user_id.to_string(), membership.to_owned())
}

/// `inviter` invites `user_id` to the room, who accepts when a token is given for them.
async fn add(
    service: &Service,
    inviter: &str,
    room_id: &RoomId,
    user_id: &UserId,
    accept_with: Option<&str>,
) {
    client_api(
        service,
        inviter,
        &format!("rooms/{room_id}/invite"),
        json!({"user_id": user_id}),
    )
    .await;
    if let Some(token) = accept_with {
        client_api(service, token, &format!("rooms/{room_id}/join"), json!({})).await;
    }
}

const NAMED_INVITER: &str = "sliding-invitation-named-inviter";
const NAMED_FIRST: &str = "sliding-invitation-named-first";
const NAMED_SECOND: &str = "sliding-invitation-named-second";
const NAMED_GONE: &str = "sliding-invitation-named-gone";
const NAMED_PENDING: &str = "sliding-invitation-named-pending";
const NAMED_INVITEE: &str = "sliding-invitation-named-invitee";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
fn database_an_invitation_names_other_members_up_to_a_limit() {
    run_database_test("invitation-members", invitation_members);
}

async fn invitation_members() {
    configure();
    let inviter = create_user("invitation-named-inviter", NAMED_INVITER).await;
    let first = create_user("invitation-named-first", NAMED_FIRST).await;
    let second = create_user("invitation-named-second", NAMED_SECOND).await;
    let gone = create_user("invitation-named-gone", NAMED_GONE).await;
    let pending = create_user("invitation-named-pending", NAMED_PENDING).await;
    let invitee = create_user("invitation-named-invitee", NAMED_INVITEE).await;
    let service = Service::new(crate::routing::root());

    // `first` joins. `pending` is invited and does not answer. After that invitation `second`
    // joins, and `gone` joins and leaves.
    let room_id = invite(&service, NAMED_INVITER, &first).await;
    client_api(
        &service,
        NAMED_FIRST,
        &format!("rooms/{room_id}/join"),
        json!({}),
    )
    .await;
    add(&service, NAMED_INVITER, &room_id, &pending, None).await;
    add(
        &service,
        NAMED_INVITER,
        &room_id,
        &second,
        Some(NAMED_SECOND),
    )
    .await;
    add(&service, NAMED_INVITER, &room_id, &gone, Some(NAMED_GONE)).await;
    client_api(
        &service,
        NAMED_GONE,
        &format!("rooms/{room_id}/leave"),
        json!({}),
    )
    .await;
    let start = sync(&service, NAMED_INVITEE, None).await;
    add(&service, NAMED_INVITER, &room_id, &invitee, None).await;

    // A server that is not configured to name members sends the inviter's membership and the
    // invitation, as before.
    let invited = sync(&service, NAMED_INVITEE, Some(pos(&start))).await;
    assert_eq!(
        members(&invited["rooms"][room_id.as_str()]["invite_state"]),
        [membership(&inviter, "join"), membership(&invitee, "invite")],
        "{invited}"
    );

    let invitation = room::get_state(
        &room_id,
        &StateEventType::RoomMember,
        invitee.as_str(),
        None,
    )
    .await
    .unwrap();
    // Joined members come before invited ones, each from the earliest. A user who left is
    // not a member, and neither the inviter nor the invited user is named twice.
    let others = [
        membership(&first, "join"),
        membership(&second, "join"),
        membership(&pending, "invite"),
    ];
    for (limit, named) in [(0, 0), (1, 1), (2, 2), (3, 3), (usize::MAX, 3)] {
        let stripped = state::invitation_stripped_with(&invitation, limit)
            .await
            .unwrap();
        let stripped = serde_json::to_value(&stripped).unwrap();
        let mut expected = vec![membership(&inviter, "join")];
        expected.extend_from_slice(&others[..named]);
        expected.push(membership(&invitee, "invite"));
        assert_eq!(members(&stripped), expected, "limit {limit}: {stripped}");
        // The invitation stays the last event.
        let last = stripped.as_array().unwrap().last().unwrap();
        assert_eq!(
            last["state_key"],
            json!(invitee),
            "limit {limit}: {stripped}"
        );
        assert!(
            !stripped.to_string().contains(gone.as_str()),
            "limit {limit}: {stripped}"
        );
    }
}

const LISTED_OWNER: &str = "sliding-invitation-listed-owner";
const LISTED_FIRST: &str = "sliding-invitation-listed-first";
const LISTED_SECOND: &str = "sliding-invitation-listed-second";
const LISTED_THIRD: &str = "sliding-invitation-listed-third";
const LISTED_INVITEE: &str = "sliding-invitation-listed-invitee";
const LISTED_KNOCKER: &str = "sliding-invitation-listed-knocker";

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
fn review_invitation_names_room_members_when_configured() {
    run_database_test(
        "invitation-members-configured",
        configured_invitation_members,
    );
}

async fn configured_invitation_members() {
    crate::test_database::init();
    crate::config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name": "invitation.example", "db": {"url": "unused-test-config"},
                "ip_range_denylist": [], "invite_state_member_limit": 2,
            }))
            .unwrap(),
        )
        .expect("run alone");
    let owner = create_user("invitation-listed-owner", LISTED_OWNER).await;
    let first = create_user("invitation-listed-first", LISTED_FIRST).await;
    let second = create_user("invitation-listed-second", LISTED_SECOND).await;
    let third = create_user("invitation-listed-third", LISTED_THIRD).await;
    let invitee = create_user("invitation-listed-invitee", LISTED_INVITEE).await;
    let knocker = create_user("invitation-listed-knocker", LISTED_KNOCKER).await;
    let service = Service::new(crate::routing::root());

    let room_id = invite(&service, LISTED_OWNER, &first).await;
    client_api(
        &service,
        LISTED_FIRST,
        &format!("rooms/{room_id}/join"),
        json!({}),
    )
    .await;
    add(
        &service,
        LISTED_OWNER,
        &room_id,
        &second,
        Some(LISTED_SECOND),
    )
    .await;
    add(&service, LISTED_OWNER, &room_id, &third, Some(LISTED_THIRD)).await;
    let start = sync(&service, LISTED_INVITEE, None).await;
    add(&service, LISTED_OWNER, &room_id, &invitee, None).await;

    // The invitation names the two earliest of the three other members, in sliding sync and
    // in `/sync`.
    let expected = [
        membership(&owner, "join"),
        membership(&first, "join"),
        membership(&second, "join"),
        membership(&invitee, "invite"),
    ];
    let invited = sync(&service, LISTED_INVITEE, Some(pos(&start))).await;
    assert_eq!(
        members(&invited["rooms"][room_id.as_str()]["invite_state"]),
        expected,
        "{invited}"
    );
    let mut response = send(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            format!("Bearer {LISTED_INVITEE}"),
            true,
        ),
        &service,
    )
    .await;
    let classic = response.take_json::<Value>().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{classic}");
    assert_eq!(
        members(&classic["rooms"]["invite"][room_id.as_str()]["invite_state"]["events"]),
        expected,
        "{classic}"
    );

    // Such an invitation is accepted like any other.
    client_api(
        &service,
        LISTED_INVITEE,
        &format!("rooms/{room_id}/join"),
        json!({}),
    )
    .await;
    let joined = sync(&service, LISTED_INVITEE, Some(pos(&invited))).await;
    assert_joined_in_full(&joined["rooms"][room_id.as_str()], &invitee);

    // Nobody in a room asks for a knock, so the state kept with a knock names no member of
    // the room.
    let created = client_api(
        &service,
        LISTED_OWNER,
        "createRoom",
        json!({
            "room_version": "11",
            "preset": "private_chat",
            "name": "Knock room",
            "initial_state": [{
                "type": "m.room.join_rules",
                "state_key": "",
                "content": {"join_rule": "knock"},
            }],
        }),
    )
    .await;
    let knock_room = OwnedRoomId::try_from(created["room_id"].as_str().unwrap()).unwrap();
    add(
        &service,
        LISTED_OWNER,
        &knock_room,
        &first,
        Some(LISTED_FIRST),
    )
    .await;
    client_api(
        &service,
        LISTED_KNOCKER,
        &format!("knock/{knock_room}"),
        json!({}),
    )
    .await;
    let knock = room::user::current_membership(&knocker, &knock_room)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(knock.membership, MembershipState::Knock);
    let kept = knock.stripped_state.unwrap();
    assert!(!members(&kept).is_empty(), "{kept}");
    for (user_id, _) in members(&kept) {
        assert!(
            user_id == knocker.as_str() || user_id == owner.as_str(),
            "{kept}"
        );
    }
    assert!(!kept.to_string().contains(first.as_str()), "{kept}");
}
