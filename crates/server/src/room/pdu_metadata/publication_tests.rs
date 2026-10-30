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
}
