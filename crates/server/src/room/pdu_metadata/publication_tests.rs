use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use super::*;
use crate::core::{OwnedDeviceId, OwnedRoomId, RoomVersionId, UnixMillis};
use crate::event::PduBuilder;

const SERVER: &str = "relation-publication.example";

fn token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(|| {
        format!(
            "q617-relations-{}",
            ulid::Ulid::generate().to_string().to_lowercase()
        )
    })
}

fn authorization() -> String {
    format!("Bearer {}", token())
}

async fn request(request: RequestBuilder, service: &Service) -> Value {
    let router = service.router();
    let mut response = tokio::spawn(async move { request.send(router).await })
        .await
        .unwrap();
    let body: Value = response.take_json().await.unwrap();
    assert_eq!(response.status_code, Some(StatusCode::OK), "{body}");
    body
}

async fn send(service: &Service, room: &RoomId, transaction: &str, content: Value) -> OwnedEventId {
    let body = request(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/{transaction}"
        ))
        .add_header("Authorization", authorization(), true)
        .json(&content),
        service,
    )
    .await;
    body["event_id"].as_str().unwrap().try_into().unwrap()
}

fn relation(root: &EventId, body: &str) -> Value {
    json!({"msgtype":"m.text", "body":body,
        "m.relates_to":{"rel_type":"m.thread", "event_id":root}})
}

async fn sync(service: &Service) -> String {
    let body = request(
        TestClient::get("http://localhost/_matrix/client/v3/sync?timeout=0").add_header(
            "Authorization",
            authorization(),
            true,
        ),
        service,
    )
    .await;
    let token = body["next_batch"].as_str().unwrap().to_owned();
    assert!(matches!(
        token.parse::<BatchToken>().unwrap(),
        BatchToken::Publication { .. }
    ));
    token
}

async fn page(
    service: &Service,
    room: &RoomId,
    root: &EventId,
    suffix: &str,
    from: &str,
    to: Option<&str>,
    direction: &str,
    limit: usize,
) -> Value {
    let mut url = url::Url::parse(&format!(
        "http://localhost/_matrix/client/v3/rooms/{room}/relations/{root}{suffix}"
    ))
    .unwrap();
    url.query_pairs_mut()
        .append_pair("from", from)
        .append_pair("dir", direction)
        .append_pair("limit", &limit.to_string());
    if let Some(to) = to {
        url.query_pairs_mut().append_pair("to", to);
    }
    request(
        TestClient::get(url.as_str()).add_header("Authorization", authorization(), true),
        service,
    )
    .await
}

fn ids(body: &Value) -> Vec<String> {
    body["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["event_id"].as_str().unwrap().to_owned())
        .collect()
}

async fn publication(id: &EventId) -> Seqnum {
    data::schema::event_points::table
        .find(id)
        .select(data::schema::event_points::live_published_sn)
        .first::<Option<Seqnum>>(&mut data::connect().await.unwrap())
        .await
        .unwrap()
        .unwrap()
}

async fn promote(
    room: &RoomId,
    pdu: &SnPduEvent,
    mut wire: crate::core::serde::CanonicalJsonObject,
) {
    wire.remove("event_id");
    crate::event::handler::process_incoming_pdu(
        crate::config::server_name(),
        &pdu.event_id,
        room,
        &RoomVersionId::V11,
        wire,
        true,
        false,
    )
    .await
    .unwrap();
    let accepted = timeline::get_pdu(&pdu.event_id).await.unwrap();
    assert!(!accepted.is_outlier && !accepted.soft_failed);
    assert_eq!(
        accepted.event_sn, pdu.event_sn,
        "promotion rewrote identity"
    );
}

#[test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
fn database_relations_preserve_publication_windows_and_legacy_identity_pages() {
    std::thread::Builder::new()
        .name("relations-publication-http".into())
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
            "server_name":SERVER, "db":{"url":"unused-test-config"},
            "ip_range_denylist":[], "auto_join_rooms":[]
        }))
        .unwrap()
    });
    let user = UserId::parse(format!("@{}:{}", token(), crate::config::server_name())).unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: user.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: user.localpart().into(),
        server_name: user.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::device::create_device(&user, &OwnedDeviceId::from("D"), token(), None, None)
        .await
        .unwrap();
    crate::user::get_push_rules(&user).await.unwrap();
    let service = Service::new(crate::routing::root());
    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", authorization(), true)
            .json(&json!({"room_version":"11", "preset":"public_chat"})),
        &service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let root = send(
        &service,
        &room,
        "root",
        json!({"msgtype":"m.text", "body":"root"}),
    )
    .await;
    let mut pending = Vec::new();
    for name in ["late-before-upper", "future-after-upper"] {
        let (pdu, wire) = PduBuilder {
            event_type: "m.room.message".into(),
            content: serde_json::value::to_raw_value(&relation(&root, name)).unwrap(),
            ..Default::default()
        }
        .hash_sign(&user, &room, &RoomVersionId::V11)
        .await
        .unwrap();
        let (pdu, wire, guard) = PduBuilder::save_as_outlier(pdu, wire, &user).await.unwrap();
        drop(guard);
        pending.push((pdu, wire));
    }
    let earlier = send(
        &service,
        &room,
        "earlier",
        relation(&root, "earlier publication"),
    )
    .await;
    let lower = sync(&service).await;
    let lower_sn = lower.parse::<BatchToken>().unwrap().stream_ordering();
    for (cached, _) in &pending {
        assert!(cached.event_sn < lower_sn);
    }
    promote(&room, &pending[0].0, pending[0].1.clone()).await;
    assert!(publication(&pending[0].0.event_id).await >= lower_sn);
    let newer = send(
        &service,
        &room,
        "newer",
        relation(&root, "newer publication"),
    )
    .await;
    let upper = sync(&service).await;
    let upper_sn = upper.parse::<BatchToken>().unwrap().stream_ordering();

    // All three actual handlers share publication-aware selection before LIMIT.
    for suffix in ["", "/m.thread", "/m.thread/m.room.message"] {
        let body = page(
            &service,
            &room,
            &root,
            suffix,
            &upper,
            Some(&lower),
            "b",
            100,
        )
        .await;
        assert_eq!(
            ids(&body),
            vec![newer.to_string(), pending[0].0.event_id.to_string()],
            "{body}"
        );
    }
    let first = page(&service, &room, &root, "", &upper, Some(&lower), "b", 1).await;
    assert_eq!(ids(&first), vec![newer.to_string()]);
    let relation_end = first["next_batch"].as_str().unwrap().to_owned();
    assert!(matches!(relation_end.parse::<BatchToken>().unwrap(),
        BatchToken::PublicationPage { lower:Some(l), upper:Some(u), .. } if l==lower_sn && u==upper_sn));

    // /relations accepts the actual qualified continuation returned by /messages.
    let mut messages = url::Url::parse(&format!(
        "http://localhost/_matrix/client/v3/rooms/{room}/messages"
    ))
    .unwrap();
    messages
        .query_pairs_mut()
        .append_pair("from", &upper)
        .append_pair("to", &lower)
        .append_pair("dir", "b")
        .append_pair("limit", "1");
    let message_page = request(
        TestClient::get(messages.as_str()).add_header("Authorization", authorization(), true),
        &service,
    )
    .await;
    assert_eq!(ids(&message_page), vec![newer.to_string()]);
    let message_end = message_page["end"].as_str().unwrap().to_owned();
    assert!(
        message_end
            .parse::<BatchToken>()
            .unwrap()
            .is_publication_page()
    );

    // This old identity lies inside the remaining topology page, but was only
    // published after the frozen upper bound. Both kinds of continuation must
    // exclude it while still returning the older in-window child.
    promote(&room, &pending[1].0, pending[1].1.clone()).await;
    assert!(publication(&pending[1].0.event_id).await >= upper_sn);
    for from in [&relation_end, &message_end] {
        let second = page(&service, &room, &root, "", from, None, "b", 1).await;
        assert_eq!(
            ids(&second),
            vec![pending[0].0.event_id.to_string()],
            "{second}"
        );
        let end = second["next_batch"].as_str().unwrap();
        assert_ne!(from, end);
        let exhausted = page(&service, &room, &root, "", end, None, "b", 1).await;
        assert!(
            ids(&exhausted).is_empty(),
            "future/earlier child escaped frozen window: {exhausted}"
        );
    }
    let forward = page(&service, &room, &root, "", &lower, Some(&upper), "f", 1).await;
    assert_eq!(ids(&forward), vec![pending[0].0.event_id.to_string()]);
    let forward_end = forward["next_batch"].as_str().unwrap();
    assert!(
        forward_end
            .parse::<BatchToken>()
            .unwrap()
            .is_publication_page()
    );
    let forward_next = page(&service, &room, &root, "", forward_end, None, "f", 1).await;
    assert_eq!(ids(&forward_next), vec![newer.to_string()]);
    let forward_done = page(
        &service,
        &room,
        &root,
        "",
        forward_next["next_batch"].as_str().unwrap(),
        None,
        "f",
        1,
    )
    .await;
    assert!(ids(&forward_done).is_empty());

    // Old s/t entry points retain original identity ordering and s continuation.
    let mut legacy = Vec::new();
    for id in [
        &*earlier,
        &*newer,
        &*pending[0].0.event_id,
        &*pending[1].0.event_id,
    ] {
        let pdu = timeline::get_pdu(id).await.unwrap();
        legacy.push((pdu.event_sn, id.to_string()));
    }
    legacy.sort_by_key(|(sn, _)| std::cmp::Reverse(*sn));
    let expected: Vec<_> = legacy.into_iter().map(|(_, id)| id).collect();
    for from in [
        BatchToken::LIVE_MAX.to_string(),
        BatchToken::new_historic(Seqnum::MAX, 999).to_string(),
    ] {
        let body = page(&service, &room, &root, "", &from, None, "b", 100).await;
        assert_eq!(ids(&body), expected, "legacy pagination changed: {body}");
        assert!(matches!(
            body["next_batch"]
                .as_str()
                .unwrap()
                .parse::<BatchToken>()
                .unwrap(),
            BatchToken::Live { .. }
        ));
    }
    mixed_identity_windows(&service, &user).await;
}

#[test]
fn relation_legacy_bounds_keep_absolute_inclusive_child_axis() {
    for direction in [Direction::Forward, Direction::Backward] {
        for from in [true, false] {
            let mut window = timeline::publication::Window {
                identity_lower: Some(-100),
                identity_upper: Some(-10),
                ..Default::default()
            };
            assert!(
                relation_bound(BatchToken::new_live(-7), from, direction, &mut window)
                    .unwrap()
                    .is_none()
            );
            if (direction == Direction::Forward) == from {
                assert_eq!(window.child_lower, Some(7));
                assert_eq!(window.child_upper, None);
            } else {
                assert_eq!(window.child_lower, None);
                assert_eq!(window.child_upper, Some(8));
            }
            assert_eq!(
                (window.identity_lower, window.identity_upper),
                (Some(-100), Some(-10))
            );
            assert_eq!((window.lower, window.upper), (None, None));
        }
    }
    let mut window = timeline::publication::Window::default();
    relation_bound(
        BatchToken::new_live(Seqnum::MAX),
        true,
        Direction::Backward,
        &mut window,
    )
    .unwrap();
    assert_eq!(
        window.child_upper, None,
        "inclusive MAX is unbounded, never MAX+1 overflow"
    );
    assert!(
        relation_bound(
            BatchToken::new_live(Seqnum::MIN),
            true,
            Direction::Forward,
            &mut window
        )
        .is_err()
    );
}

async fn messages_page(
    service: &Service,
    room: &RoomId,
    from: &str,
    to: Option<&str>,
    direction: &str,
    limit: usize,
) -> Value {
    let mut url = url::Url::parse(&format!(
        "http://localhost/_matrix/client/v3/rooms/{room}/messages"
    ))
    .unwrap();
    url.query_pairs_mut()
        .append_pair("from", from)
        .append_pair("dir", direction)
        .append_pair("limit", &limit.to_string());
    if let Some(to) = to {
        url.query_pairs_mut().append_pair("to", to);
    }
    request(
        TestClient::get(url.as_str()).add_header("Authorization", authorization(), true),
        service,
    )
    .await
}

async fn mixed_identity_windows(service: &Service, user: &UserId) {
    let created = request(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .add_header("Authorization", authorization(), true)
            .json(&json!({"room_version":"11","preset":"public_chat"})),
        service,
    )
    .await;
    let room: OwnedRoomId = created["room_id"].as_str().unwrap().try_into().unwrap();
    let root = send(
        service,
        &room,
        "mixed-root",
        json!({"msgtype":"m.text","body":"mixed root"}),
    )
    .await;
    // Sign a valid lower-depth branch now, but defer its identity allocation.
    let (low_pdu, low_wire) = PduBuilder {
        event_type: "m.room.message".into(),
        content: serde_json::value::to_raw_value(&relation(&root, "lower depth, later identity"))
            .unwrap(),
        ..Default::default()
    }
    .hash_sign(user, &room, &RoomVersionId::V11)
    .await
    .unwrap();
    for step in ["advance-a", "advance-b"] {
        send(
            service,
            &room,
            step,
            json!({"msgtype":"m.text","body":step}),
        )
        .await;
    }
    let (old_pdu, old_wire) = PduBuilder {
        event_type: "m.room.message".into(),
        content: serde_json::value::to_raw_value(&relation(&root, "higher depth, older identity"))
            .unwrap(),
        ..Default::default()
    }
    .hash_sign(user, &room, &RoomVersionId::V11)
    .await
    .unwrap();
    let (old, old_wire, guard) = PduBuilder::save_as_outlier(old_pdu, old_wire, user)
        .await
        .unwrap();
    drop(guard);
    let (low, low_wire, guard) = PduBuilder::save_as_outlier(low_pdu, low_wire, user)
        .await
        .unwrap();
    drop(guard);
    assert!(
        old.event_sn < low.event_sn && old.depth > low.depth,
        "fixture must create real identity/topology inversion"
    );
    promote(&room, &low, low_wire).await;
    promote(&room, &old, old_wire).await;
    let last = send(
        service,
        &room,
        "mixed-last",
        relation(&root, "last valid identity"),
    )
    .await;
    let last_pdu = timeline::get_pdu(&last).await.unwrap();
    let upper = sync(service).await;
    let upper_sn = upper.parse::<BatchToken>().unwrap().stream_ordering();
    assert!(publication(&old.event_id).await < upper_sn);
    assert!(publication(&low.event_id).await < upper_sn);
    let positive = BatchToken::new_live(low.event_sn).to_string();
    let negative = BatchToken::new_live(-low.event_sn).to_string();
    for from in [&positive, &negative] {
        let first = page(service, &room, &root, "", from, Some(&upper), "f", 1).await;
        assert_eq!(
            ids(&first),
            vec![low.event_id.to_string()],
            "legacy inclusive child identity changed: {first}"
        );
        let continuation = first["next_batch"].as_str().unwrap();
        assert!(matches!(continuation.parse::<BatchToken>().unwrap(),
            BatchToken::PublicationPage { child_lower:Some(lower), identity_lower:None, .. } if lower==low.event_sn));
        let second = page(service, &room, &root, "", continuation, None, "f", 1).await;
        assert_eq!(
            ids(&second),
            vec![last.to_string()],
            "q lost child identity floor and exposed older identity: {second}"
        );
        let done = page(
            service,
            &room,
            &root,
            "",
            second["next_batch"].as_str().unwrap(),
            None,
            "f",
            1,
        )
        .await;
        assert!(ids(&done).is_empty());
        let backwards = page(service, &room, &root, "", &upper, Some(from), "b", 1).await;
        assert_eq!(ids(&backwards), vec![last.to_string()]);
        let back_end = backwards["next_batch"].as_str().unwrap();
        let back_second = page(service, &room, &root, "", back_end, None, "b", 1).await;
        assert_eq!(
            ids(&back_second),
            vec![low.event_id.to_string()],
            "q lost backward child identity floor"
        );
        let cross = messages_page(service, &room, back_end, None, "b", 100).await;
        assert_eq!(
            ids(&cross),
            vec![low.event_id.to_string()],
            "/messages did not retain relation child axis"
        );
        let legacy = page(
            service,
            &room,
            &root,
            "",
            from,
            Some(&BatchToken::LIVE_MAX.to_string()),
            "f",
            100,
        )
        .await;
        assert_eq!(
            ids(&legacy),
            vec![low.event_id.to_string(), last.to_string()]
        );
        assert!(matches!(
            legacy["next_batch"]
                .as_str()
                .unwrap()
                .parse::<BatchToken>()
                .unwrap(),
            BatchToken::Live { .. }
        ));
    }
    // A messages q carries signed-stream identity bounds. Adding a relations
    // s endpoint must intersect its separate child-SN axis without overwriting it.
    let message = messages_page(service, &room, &positive, Some(&upper), "f", 1).await;
    assert_eq!(ids(&message), vec![low.event_id.to_string()]);
    let message_end = message["end"].as_str().unwrap();
    let cap = BatchToken::new_live(-last_pdu.event_sn).to_string();
    let intersected = page(service, &room, &root, "", message_end, Some(&cap), "f", 1).await;
    assert_eq!(ids(&intersected), vec![last.to_string()]);
    let both_end = intersected["next_batch"].as_str().unwrap();
    assert!(
        matches!(both_end.parse::<BatchToken>().unwrap(), BatchToken::PublicationPage {
        identity_lower:Some(lower), child_upper:Some(cap), ..
    } if lower==low.event_sn && cap==last_pdu.event_sn+1)
    );
    let reverse = messages_page(service, &room, both_end, None, "b", 100).await;
    assert_eq!(
        ids(&reverse),
        vec![last.to_string(), low.event_id.to_string()],
        "cross-endpoint q dropped one identity axis"
    );

    // Import genuine negative history through the authenticated backfill path.
    let (history, history_wire) = PduBuilder {
        event_type: "m.room.message".into(),
        content: serde_json::value::to_raw_value(&relation(&root, "negative historical child"))
            .unwrap(),
        ..Default::default()
    }
    .hash_sign(user, &room, &RoomVersionId::V11)
    .await
    .unwrap();
    let (history, mut history_wire, guard) =
        PduBuilder::save_as_outlier(history, history_wire, user)
            .await
            .unwrap();
    drop(guard);
    history_wire.remove("event_id");
    crate::event::handler::process_incoming_pdu(
        crate::config::server_name(),
        &history.event_id,
        &room,
        &RoomVersionId::V11,
        history_wire,
        true,
        true,
    )
    .await
    .unwrap();
    let history = timeline::get_pdu(&history.event_id).await.unwrap();
    assert!(history.is_backfill && !history.is_outlier && !history.soft_failed);
    let stored = data::schema::events::table
        .find(&*history.event_id)
        .select(data::schema::events::stream_ordering)
        .first::<Seqnum>(&mut data::connect().await.unwrap())
        .await
        .unwrap();
    assert_eq!(stored, -history.event_sn);
    // Backfill intentionally avoids live relation aggregation. Populate its
    // relation index using the same production metadata writer as live imports.
    add_relation(&room, &root, &history.event_id, Some(RelationType::Thread))
        .await
        .unwrap();
    let legacy_history = BatchToken::new_live(-history.event_sn).to_string();
    let qualified_history = page(
        service,
        &room,
        &root,
        "",
        &legacy_history,
        Some(&upper),
        "f",
        100,
    )
    .await;
    assert_eq!(
        ids(&qualified_history),
        vec![history.event_id.to_string()],
        "absolute child lower must not become signed stream lower"
    );
    let old_history = page(service, &room, &root, "", &legacy_history, None, "b", 100).await;
    assert!(ids(&old_history).contains(&history.event_id.to_string()));
    let q = BatchToken::new_publication_page_with_bounds(
        None,
        Some(upper_sn),
        history.depth as i64,
        stored - 1,
        Some(stored),
        Some(stored + 1),
        Some(history.event_sn),
        Some(history.event_sn + 1),
    )
    .to_string();
    let exact_history = page(service, &room, &root, "", &q, None, "f", 1).await;
    assert_eq!(ids(&exact_history), vec![history.event_id.to_string()]);
    let history_end = exact_history["next_batch"].as_str().unwrap();
    assert!(
        matches!(history_end.parse::<BatchToken>().unwrap(), BatchToken::PublicationPage {
        identity_lower:Some(stream_lower), child_lower:Some(child_lower), ..
    } if stream_lower==stored && child_lower==history.event_sn)
    );
    let cross_history = messages_page(service, &room, &q, None, "f", 1).await;
    assert_eq!(ids(&cross_history), vec![history.event_id.to_string()]);
    let excluded = BatchToken::new_publication_page_with_bounds(
        None,
        Some(upper_sn),
        history.depth as i64,
        stored - 1,
        Some(stored),
        Some(stored + 1),
        Some(history.event_sn + 1),
        None,
    )
    .to_string();
    assert!(ids(&page(service, &room, &root, "", &excluded, None, "f", 1).await).is_empty());
    assert!(ids(&messages_page(service, &room, &excluded, None, "f", 1).await).is_empty());
}
