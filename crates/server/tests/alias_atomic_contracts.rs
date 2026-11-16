//! Run alone against an EMPTY disposable PALPO_TEST_DATABASE_URL.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diesel::connection::SimpleConnection;
use diesel::{Connection, PgConnection};
use palpo::core::{OwnedDeviceId, OwnedRoomId, RoomAliasId, UnixMillis, UserId};
use palpo::{config, data};
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

#[derive(Clone)]
struct AuthRead {
    armed: Arc<AtomicBool>,
    complete: Arc<Notify>,
}
impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for AuthRead {
    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        if self.armed.load(Ordering::SeqCst)
            && ctx
                .span(&id)
                .is_some_and(|s| s.metadata().name() == "user_can_remove_alias")
        {
            self.complete.notify_one();
        }
    }
}
fn sql(db: &mut PgConnection, query: &str) {
    db.batch_execute(query).unwrap();
}
#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type=diesel::sql_types::BigInt)]
    count: i64,
}
async fn wait_insert_gate(db: &mut PgConnection, expected: i64) {
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let count:Count=diesel::RunQueryDsl::get_result(diesel::sql_query("SELECT count(*) AS count FROM pg_locks WHERE locktype='advisory' AND classid=0 AND objid=117001 AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())"),db).unwrap();
            if count.count==expected {break;}
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("both HTTP requests must reach their insert after passing prechecks");
}
fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
fn alias_url(alias: &str) -> String {
    format!(
        "http://localhost/_matrix/client/v3/directory/room/{}",
        encoded(alias)
    )
}
async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        (
            response.status_code.unwrap(),
            response.take_json::<Value>().await.unwrap(),
        )
    })
    .await
    .unwrap()
}
async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let (status, body) = send(request, service).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
async fn create(service: &Service, token: &str) -> OwnedRoomId {
    let result = ok(
        TestClient::post("http://localhost/_matrix/client/v3/createRoom")
            .bearer_auth(token)
            .json(&json!({"preset":"private_chat","room_version":"11"})),
        service,
    )
    .await;
    result["room_id"].as_str().unwrap().try_into().unwrap()
}
async fn mapped(alias: &str) -> Option<OwnedRoomId> {
    data::room::get_alias_room_id(RoomAliasId::parse(alias).unwrap().as_ref())
        .await
        .unwrap()
}
#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn alias_mutations_are_atomic_and_report_mapping_conflicts() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(contracts());
        })
        .unwrap()
        .join()
        .unwrap();
}
async fn contracts() {
    let url = std::env::var("PALPO_TEST_DATABASE_URL").expect("empty dedicated database required");
    let mut control = PgConnection::establish(&url).unwrap();
    let tables:Count=diesel::RunQueryDsl::get_result(diesel::sql_query("SELECT count(*) AS count FROM pg_tables WHERE schemaname NOT IN ('pg_catalog','information_schema')"),&mut control).unwrap();
    assert_eq!(tables.count, 0, "refuse nonempty database");
    data::init(
        &serde_json::from_value(json!({"url":url,"pool_size":10,"statement_timeout":5000}))
            .unwrap(),
    )
    .unwrap();
    config::CONFIG.set(serde_json::from_value(json!({"server_name":"alias-contract.example","db":{"url":"unused"},"auto_join_rooms":[],"presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}})).unwrap()).unwrap();
    // Keep the real enqueue channel available without starting any network
    // delivery worker; AS-subscribed room events remain in this disposable DB.
    let (outgoing, _outgoing_receiver) = tokio::sync::mpsc::channel(4096);
    palpo::sending::MPSC_SENDER.set(outgoing).unwrap();
    let observed = AuthRead {
        armed: Arc::new(AtomicBool::new(false)),
        complete: Arc::new(Notify::new()),
    };
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(observed.clone()))
        .unwrap();
    for (name, admin) in [("alice", false), ("bob", false), ("admin", true)] {
        let user = UserId::parse(format!("@{name}:alias-contract.example")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: admin,
            is_guest: false,
            is_local: true,
            localpart: name.into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        data::user::ensure_profile_exists(&user).await.unwrap();
        data::user::device::create_device(
            &user,
            &OwnedDeviceId::from("D"),
            &format!("{name}-token"),
            None,
            None,
        )
        .await
        .unwrap();
    }
    let service = Service::new(palpo::routing::root());
    let room_a = create(&service, "alice-token").await;
    let room_b = create(&service, "bob-token").await;
    let mut failures = Vec::new();

    let existing = "#existing:alias-contract.example";
    ok(
        TestClient::put(alias_url(existing))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":room_a})),
        &service,
    )
    .await;
    let existing_result = send(
        TestClient::put(alias_url(existing))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":room_a})),
        &service,
    )
    .await;
    println!(
        "existing alias PUT: {} {}",
        existing_result.0, existing_result.1
    );
    if existing_result.0 != StatusCode::CONFLICT {
        failures.push("existing alias must return 409");
    }

    // A per-alias BEFORE INSERT gate lets both requests finish their absence,
    // namespace and membership checks before either wins the unique insertion.
    let race = "#claim-race:alias-contract.example";
    sql(
        &mut control,
        "CREATE FUNCTION gate_alias_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.alias_id='#claim-race:alias-contract.example' THEN PERFORM pg_advisory_xact_lock(117001); END IF; RETURN NEW; END $$; CREATE TRIGGER gate_alias_insert BEFORE INSERT ON room_aliases FOR EACH ROW EXECUTE FUNCTION gate_alias_insert(); SELECT pg_advisory_lock(117001)",
    );
    let a = TestClient::put(alias_url(race))
        .bearer_auth("alice-token")
        .json(&json!({"room_id":room_a}));
    let b = TestClient::put(alias_url(race))
        .bearer_auth("bob-token")
        .json(&json!({"room_id":room_b}));
    let router_a = service.router();
    let router_b = service.router();
    let a = tokio::spawn(async move {
        let mut r = a.send(router_a).await;
        (
            r.status_code.unwrap(),
            r.take_json::<Value>().await.unwrap(),
        )
    });
    let b = tokio::spawn(async move {
        let mut r = b.send(router_b).await;
        (
            r.status_code.unwrap(),
            r.take_json::<Value>().await.unwrap(),
        )
    });
    wait_insert_gate(&mut control, 2).await;
    sql(&mut control, "SELECT pg_advisory_unlock(117001)");
    let (a, b) = (a.await.unwrap(), b.await.unwrap());
    println!(
        "concurrent claims: {} / {}; stored {:?}",
        a.0,
        b.0,
        mapped(race).await
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    if statuses != [StatusCode::OK, StatusCode::CONFLICT] {
        failures.push("concurrent claims require one success and one conflict");
    }
    sql(
        &mut control,
        "DROP TRIGGER gate_alias_insert ON room_aliases; DROP FUNCTION gate_alias_insert()",
    );

    // createRoom has a different Matrix error contract for the same late
    // insertion conflict: HTTP 400 / M_ROOM_IN_USE, never a false 200.
    sql(
        &mut control,
        "CREATE FUNCTION gate_alias_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.alias_id='#room-create-race:alias-contract.example' THEN PERFORM pg_advisory_xact_lock(117001); END IF; RETURN NEW; END $$; CREATE TRIGGER gate_alias_insert BEFORE INSERT ON room_aliases FOR EACH ROW EXECUTE FUNCTION gate_alias_insert(); SELECT pg_advisory_lock(117001)",
    );
    let mut requests = Vec::new();
    for token in ["alice-token", "bob-token"] {
        let router = service.router();
        requests.push(tokio::spawn(async move {
            let mut response=TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .bearer_auth(token).json(&json!({"room_version":"11","preset":"private_chat","room_alias_name":"room-create-race"})).send(router).await;
            (response.status_code.unwrap(),response.take_json::<Value>().await.unwrap())
        }));
    }
    wait_insert_gate(&mut control, 2).await;
    sql(&mut control, "SELECT pg_advisory_unlock(117001)");
    let first = requests.remove(0).await.unwrap();
    let second = requests.remove(0).await.unwrap();
    let (winner, loser) = if first.0 == StatusCode::OK {
        (&first, &second)
    } else {
        (&second, &first)
    };
    assert_eq!(winner.0, StatusCode::OK);
    assert_eq!(loser.0, StatusCode::BAD_REQUEST, "{loser:?}");
    assert_eq!(loser.1["errcode"], "M_ROOM_IN_USE");
    assert_eq!(
        mapped("#room-create-race:alias-contract.example")
            .await
            .unwrap()
            .as_str(),
        winner.1["room_id"].as_str().unwrap()
    );
    sql(
        &mut control,
        "DROP TRIGGER gate_alias_insert ON room_aliases; DROP FUNCTION gate_alias_insert()",
    );
    let aba = "#aba:alias-contract.example";
    ok(
        TestClient::put(alias_url(aba))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":room_a})),
        &service,
    )
    .await;
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{}/leave",
            encoded(room_a.as_str())
        ))
        .bearer_auth("alice-token")
        .json(&json!({})),
        &service,
    )
    .await;
    let lock = palpo::room::lock_state(&room_a).await;
    observed.armed.store(true, Ordering::SeqCst);
    let router = service.router();
    let old = tokio::spawn(async move {
        let mut r = TestClient::delete(alias_url(aba))
            .bearer_auth("alice-token")
            .send(router)
            .await;
        (
            r.status_code.unwrap(),
            r.take_json::<Value>().await.unwrap(),
        )
    });
    tokio::time::timeout(Duration::from_secs(3), observed.complete.notified())
        .await
        .expect("old owner authorized the original mapping before the room-lock wait");
    observed.armed.store(false, Ordering::SeqCst);
    // The empty room's administrator-delete path clears directory mappings
    // independently of the state lock held by the ordinary alias deletion.
    ok(
        TestClient::delete(format!(
            "http://localhost/_palpo/admin/v2/rooms/{}",
            encoded(room_a.as_str())
        ))
        .bearer_auth("admin-token")
        .json(&json!({"purge":false})),
        &service,
    )
    .await;
    ok(
        TestClient::put(alias_url(aba))
            .bearer_auth("bob-token")
            .json(&json!({"room_id":room_b})),
        &service,
    )
    .await;
    drop(lock);
    let old = old.await.unwrap();
    let replacement = mapped(aba).await;
    println!(
        "stale DELETE after admin removal/recreate: {} {}; stored {:?}",
        old.0, old.1, replacement
    );
    if old.0 == StatusCode::OK || replacement.as_ref() != Some(&room_b) {
        failures.push("stale authorization must not delete the new owner's mapping");
    }

    let upgrade_room = create(&service, "alice-token").await;
    let upgrade_alias = "#upgrade:alias-contract.example";
    ok(
        TestClient::put(alias_url(upgrade_alias))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":upgrade_room})),
        &service,
    )
    .await;
    let upgraded = ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{}/upgrade",
            encoded(upgrade_room.as_str())
        ))
        .bearer_auth("alice-token")
        .json(&json!({"new_version":"11"})),
        &service,
    )
    .await;
    let new_room = upgraded["replacement_room"]
        .as_str()
        .expect("replacement room");
    let actual = mapped(upgrade_alias).await;
    println!("upgrade replacement {new_room}; alias stored {actual:?}");
    if actual.as_ref().map(|r| r.as_str()) != Some(new_room) {
        failures.push("room upgrade must move its alias to the replacement");
    }

    let alice = UserId::parse("@alice:alias-contract.example").unwrap();
    let alice_user = data::user::get_user(&alice).await.unwrap();
    let admin_user = data::user::get_user(&UserId::parse("@admin:alias-contract.example").unwrap())
        .await
        .unwrap();
    let replacement_room: OwnedRoomId = new_room.try_into().unwrap();
    let same = "#same-generation:alias-contract.example";
    ok(
        TestClient::put(alias_url(same))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":replacement_room})),
        &service,
    )
    .await;
    let alias_id = RoomAliasId::parse(same).unwrap();
    let before = data::room::get_alias(&alias_id).await.unwrap().unwrap();
    let lock = palpo::room::lock_state(&replacement_room).await;
    observed.armed.store(true, Ordering::SeqCst);
    let router = service.router();
    let stale = tokio::spawn(async move {
        let mut response = TestClient::delete(alias_url(same))
            .bearer_auth("alice-token")
            .send(router)
            .await;
        (
            response.status_code.unwrap(),
            response.take_json::<Value>().await.unwrap(),
        )
    });
    tokio::time::timeout(Duration::from_secs(3), observed.complete.notified())
        .await
        .unwrap();
    observed.armed.store(false, Ordering::SeqCst);
    assert!(data::room::remove_alias(&before).await.unwrap());
    assert!(data::room::set_alias(before.clone()).await.unwrap());
    let after = data::room::get_alias(&alias_id).await.unwrap().unwrap();
    assert_eq!(before.room_id, after.room_id);
    assert_eq!(before.created_by, after.created_by);
    assert_eq!(before.created_at, after.created_at);
    assert_ne!(
        before.generation, after.generation,
        "same-millisecond recreation receives a distinct DB revision"
    );
    drop(lock);
    assert_eq!(stale.await.unwrap().0, StatusCode::CONFLICT);
    assert!(!data::room::remove_alias(&before).await.unwrap());
    assert!(!data::room::replace_alias(&before, &before).await.unwrap());
    assert!(data::room::replace_alias(&after, &after).await.unwrap());
    let updated = data::room::get_alias(&alias_id).await.unwrap().unwrap();
    assert_ne!(
        updated.generation, after.generation,
        "even a same-value UPDATE invalidates old snapshots"
    );
    assert!(!data::room::remove_alias(&after).await.unwrap());

    // FT05 remains intact when deletion now uses a generation-fenced snapshot.
    let other = "#untouched:alias-contract.example";
    ok(
        TestClient::put(alias_url(other))
            .bearer_auth("alice-token")
            .json(&json!({"room_id":replacement_room})),
        &service,
    )
    .await;
    let canonical = format!(
        "http://localhost/_matrix/client/v3/rooms/{}/state/m.room.canonical_alias",
        encoded(replacement_room.as_str())
    );
    ok(
        TestClient::put(&canonical)
            .bearer_auth("alice-token")
            .json(&json!({"alias":same,"alt_aliases":[other],"org.example.keep":{"value":7}})),
        &service,
    )
    .await;
    ok(
        TestClient::delete(alias_url(same)).bearer_auth("alice-token"),
        &service,
    )
    .await;
    let state = ok(
        TestClient::get(&canonical).bearer_auth("alice-token"),
        &service,
    )
    .await;
    assert!(state.get("alias").is_none());
    assert_eq!(state["alt_aliases"], json!([other]));
    assert_eq!(state["org.example.keep"], json!({"value":7}));
    assert_eq!(
        send(
            TestClient::delete(alias_url(other)).bearer_auth("bob-token"),
            &service
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(mapped(other).await.as_ref(), Some(&replacement_room));
    assert!(
        palpo::room::alias::force_set_alias(
            &room_b,
            &RoomAliasId::parse(other).unwrap(),
            &alice_user
        )
        .await
        .is_err()
    );
    palpo::room::alias::force_set_alias(&room_b, &RoomAliasId::parse(other).unwrap(), &admin_user)
        .await
        .unwrap();
    assert_eq!(mapped(other).await.as_ref(), Some(&room_b));
    ok(
        TestClient::delete(alias_url(other)).bearer_auth("admin-token"),
        &service,
    )
    .await;
    assert!(mapped(other).await.is_none());
    assert_eq!(
        send(
            TestClient::put(alias_url(config::admin_alias().as_str()))
                .bearer_auth("alice-token")
                .json(&json!({"room_id":replacement_room})),
            &service
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // Namespace assertions use real AS credentials and correctly scoped aliases.
    // A second AS has a nonexclusive overlap to exercise the exclusive owner.
    for (id, token, sender, aliases) in [
        (
            "alias-bridge",
            "bridge-as-token",
            "bridge",
            json!([{"exclusive":true,"regex":"^#bridge_.*:alias-contract[.]example$"}]),
        ),
        (
            "alias-other",
            "other-as-token",
            "other",
            json!([{"exclusive":false,"regex":"^#bridge_.*:alias-contract[.]example$"},{"exclusive":true,"regex":"^#other_.*:alias-contract[.]example$"}]),
        ),
    ] {
        palpo::appservice::register_appservice(serde_json::from_value(json!({"id":id,"url":null,"as_token":token,"hs_token":format!("{id}-hs-token"),"sender_localpart":sender,"namespaces":{"users":[{"exclusive":true,"regex":format!("^@{sender}.*:alias-contract[.]example$")}],"aliases":aliases,"rooms":[]}})).unwrap()).await.unwrap();
        for local in [
            sender.to_owned(),
            format!("{sender}_one"),
            format!("{sender}_two"),
        ] {
            let user = UserId::parse(format!("@{local}:alias-contract.example")).unwrap();
            data::user::create_user(&data::user::NewDbUser {
                id: user.clone(),
                ty: None,
                is_admin: false,
                is_guest: false,
                is_local: true,
                localpart: local,
                server_name: user.server_name().to_owned(),
                appservice_id: Some(id.into()),
                created_at: UnixMillis::now(),
            })
            .await
            .unwrap();
            data::user::ensure_profile_exists(&user).await.unwrap();
        }
    }
    let bridge = "#bridge_only:alias-contract.example";
    ok(
        TestClient::put(alias_url(bridge))
            .bearer_auth("bridge-as-token")
            .json(&json!({"room_id":replacement_room})),
        &service,
    )
    .await;
    ok(
        TestClient::delete(alias_url(bridge)).bearer_auth("bridge-as-token"),
        &service,
    )
    .await;
    let dangling = "#bridge_dangling:alias-contract.example";
    ok(
        TestClient::put(alias_url(dangling))
            .bearer_auth("bridge-as-token")
            .json(&json!({"room_id":"!not-stored:remote.example"})),
        &service,
    )
    .await;
    ok(
        TestClient::delete(format!(
            "{}?user_id={}",
            alias_url(dangling),
            encoded("@bridge_two:alias-contract.example")
        ))
        .bearer_auth("bridge-as-token"),
        &service,
    )
    .await;
    assert!(
        mapped(dangling).await.is_none(),
        "namespace control can remove a mapping without local room state"
    );
    for (label, request) in [
        (
            "AS cannot create outside its alias namespace",
            TestClient::put(alias_url("#outside:alias-contract.example"))
                .bearer_auth("bridge-as-token")
                .json(&json!({"room_id":replacement_room})),
        ),
        (
            "normal user cannot claim exclusive AS alias",
            TestClient::put(alias_url("#bridge_human:alias-contract.example"))
                .bearer_auth("alice-token")
                .json(&json!({"room_id":replacement_room})),
        ),
        (
            "client admin cannot claim exclusive AS alias",
            TestClient::put(alias_url("#bridge_admin:alias-contract.example"))
                .bearer_auth("admin-token")
                .json(&json!({"room_id":replacement_room})),
        ),
        (
            "other AS nonexclusive overlap cannot claim exclusive alias",
            TestClient::put(alias_url("#bridge_other:alias-contract.example"))
                .bearer_auth("other-as-token")
                .json(&json!({"room_id":replacement_room})),
        ),
        (
            "AS createRoom cannot claim outside namespace",
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .bearer_auth("bridge-as-token")
                .json(&json!({"room_version":"11","room_alias_name":"outside-create"})),
        ),
        (
            "normal createRoom cannot claim exclusive AS alias",
            TestClient::post("http://localhost/_matrix/client/v3/createRoom")
                .bearer_auth("alice-token")
                .json(&json!({"room_version":"11","room_alias_name":"bridge_create"})),
        ),
    ] {
        let (status, body) = send(request, &service).await;
        println!("{label}: {status} {body}");
        if !status.is_client_error() || body["errcode"] != "M_EXCLUSIVE" {
            failures.push(label);
        }
    }
    for (local, token, label) in [
        (
            "human-delete",
            "alice-token",
            "normal user cannot delete exclusive AS alias",
        ),
        (
            "admin-delete",
            "admin-token",
            "client admin cannot delete exclusive AS alias",
        ),
        (
            "other-delete",
            "other-as-token",
            "other AS cannot delete exclusive alias despite nonexclusive overlap",
        ),
    ] {
        let alias = format!("#bridge_{local}:alias-contract.example");
        ok(
            TestClient::put(format!(
                "{}?user_id={}",
                alias_url(&alias),
                encoded("@bridge_one:alias-contract.example")
            ))
            .bearer_auth("bridge-as-token")
            .json(&json!({"room_id":replacement_room})),
            &service,
        )
        .await;
        let (status, body) = send(
            TestClient::delete(alias_url(&alias)).bearer_auth(token),
            &service,
        )
        .await;
        println!("{label}: {status} {body}");
        if !status.is_client_error() || body["errcode"] != "M_EXCLUSIVE" {
            failures.push(label);
        }
    }
    let shared = "#bridge_shared:alias-contract.example";
    ok(
        TestClient::put(format!(
            "{}?user_id={}",
            alias_url(shared),
            encoded("@bridge_one:alias-contract.example")
        ))
        .bearer_auth("bridge-as-token")
        .json(&json!({"room_id":replacement_room})),
        &service,
    )
    .await;
    let result = send(
        TestClient::delete(format!(
            "{}?user_id={}",
            alias_url(shared),
            encoded("@bridge_two:alias-contract.example")
        ))
        .bearer_auth("bridge-as-token"),
        &service,
    )
    .await;
    println!(
        "same AS cross-virtual-user delete: {} {}",
        result.0, result.1
    );
    if result.0 != StatusCode::OK {
        failures.push("AS can manage its namespace across virtual users");
    }
    // Outside a nonexclusive namespace, DELETE still uses ordinary creator
    // permission; the AS creation-only restriction is not a deletion ban.
    let legacy = RoomAliasId::parse("#legacy-owned:alias-contract.example").unwrap();
    assert!(
        data::room::set_alias(data::room::DbRoomAlias {
            alias_id: legacy.clone(),
            room_id: replacement_room.clone(),
            created_by: UserId::parse("@bridge_one:alias-contract.example").unwrap(),
            created_at: UnixMillis::now(),
            generation: 0
        })
        .await
        .unwrap()
    );
    ok(
        TestClient::delete(format!(
            "{}?user_id={}",
            alias_url(legacy.as_str()),
            encoded("@bridge_one:alias-contract.example")
        ))
        .bearer_auth("bridge-as-token"),
        &service,
    )
    .await;
    assert!(failures.is_empty(), "alias contract defects: {failures:?}");
}
