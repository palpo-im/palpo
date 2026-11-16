//! Run alone against an empty disposable database: owns process-wide config/pools.
use diesel::prelude::*;
use diesel_async::SimpleAsyncConnection;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{RequestBuilder, ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, RoomAliasId, RoomId, UnixMillis, UserId};
use crate::data::schema::*;
use crate::{config, data};

async fn send(request: RequestBuilder, service: &Service) -> (StatusCode, Value) {
    let router = service.router();
    tokio::spawn(async move {
        let mut response = request.send(router).await;
        let status = response.status_code.unwrap();
        let body = response.take_json().await.unwrap();
        (status, body)
    })
    .await
    .unwrap()
}

fn admin(request: RequestBuilder) -> RequestBuilder {
    request.add_header("Authorization", "Bearer admin-contract-token", true)
}

async fn ok(request: RequestBuilder, service: &Service) -> Value {
    let (status, body) = send(request, service).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[test]
#[ignore = "run alone with an empty dedicated PALPO_TEST_DATABASE_URL"]
fn review_admin_operations_report_committed_results_and_failures() {
    std::thread::Builder::new()
        .name("admin-contract-http".into())
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
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"admin-contract.example", "db":{"url":"unused"},
                "ip_range_denylist":[], "admin":{"mas_secret":"admin-contract-mas"},
                "presence":{"allow_local":false,"allow_incoming":false,"allow_outgoing":false}
            }))
            .unwrap(),
        )
        .unwrap();
    let owner = UserId::parse("@admin:admin-contract.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: owner.clone(),
        ty: None,
        is_admin: true,
        is_guest: false,
        is_local: true,
        localpart: "admin".into(),
        server_name: owner.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&owner).await.unwrap();
    for (id, token) in [
        ("D", "admin-contract-token"),
        ("D2", "second-token"),
        ("D3", "third-token"),
    ] {
        data::user::device::create_device(&owner, &OwnedDeviceId::from(id), token, None, None)
            .await
            .unwrap();
    }
    let service = Service::new(crate::routing::root());
    let admin_devices = format!("http://localhost/_synapse/admin/v2/users/{owner}/delete_devices");
    let mas_devices = "http://localhost/_synapse/mas/sync_devices";
    let fault = "CREATE FUNCTION reject_device_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF OLD.device_id = 'D3' THEN RAISE EXCEPTION 'injected device delete failure'; END IF; RETURN OLD; END $$; CREATE TRIGGER reject_device_delete BEFORE DELETE ON user_devices FOR EACH ROW EXECUTE FUNCTION reject_device_delete();";
    data::db()
        .transaction::<_, data::DataError, _>(async |conn| {
            conn.batch_execute(fault).await?;
            Ok(())
        })
        .await
        .unwrap();
    for request in [
        admin(TestClient::post(&admin_devices)).json(&json!({"devices":["D2","D3"]})),
        TestClient::post(mas_devices)
            .add_header("Authorization", "Bearer admin-contract-mas", true)
            .json(&json!({"localpart":"admin","devices":["D"]})),
    ] {
        let (status, body) = send(request, &service).await;
        assert!(status.is_server_error(), "{status}: {body}");
        for id in ["D2", "D3"] {
            assert!(
                data::user::device::is_device_exists(&owner, &OwnedDeviceId::from(id))
                    .await
                    .unwrap()
            );
        }
        assert!(
            data::user::authenticate_token("second-token", std::time::Duration::ZERO)
                .await
                .unwrap()
                .is_some()
        );
    }
    data::db().transaction::<_,data::DataError,_>(async |conn| {
        conn.batch_execute("DROP TRIGGER reject_device_delete ON user_devices; DROP FUNCTION reject_device_delete();").await?;
        Ok(())
    }).await.unwrap();
    ok(
        admin(TestClient::post(&admin_devices)).json(&json!({"devices":["D2","missing","D2"]})),
        &service,
    )
    .await;
    assert!(
        data::user::authenticate_token("second-token", std::time::Duration::ZERO)
            .await
            .unwrap()
            .is_none()
    );
    ok(
        TestClient::post(mas_devices)
            .add_header("Authorization", "Bearer admin-contract-mas", true)
            .json(&json!({"localpart":"admin","devices":["D"]})),
        &service,
    )
    .await;
    assert!(
        !data::user::device::is_device_exists(&owner, &OwnedDeviceId::from("D3"))
            .await
            .unwrap()
    );

    let created = ok(
        admin(TestClient::post(
            "http://localhost/_matrix/client/v3/createRoom",
        ))
        .json(&json!({"room_version":"11","preset":"public_chat","room_alias_name":"delete-me"})),
        &service,
    )
    .await;
    let room = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
    let alias = RoomAliasId::parse("#delete-me:admin-contract.example").unwrap();
    ok(
        admin(TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{room}/send/m.room.message/message"
        )))
        .json(&json!({"msgtype":"m.text","body":"history to purge"})),
        &service,
    )
    .await;
    let (status, body) = send(
        admin(TestClient::get(format!(
            "http://localhost/_synapse/admin/v1/room/{room}/media"
        ))),
        &service,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["errcode"], "M_UNRECOGNIZED");

    data::db().transaction::<_,data::DataError,_>(async |conn| {
        conn.batch_execute("CREATE FUNCTION reject_history_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected purge failure'; END $$; CREATE TRIGGER reject_history_delete BEFORE DELETE ON event_datas FOR EACH ROW EXECUTE FUNCTION reject_history_delete();").await?;
        Ok(())
    }).await.unwrap();
    let delete_url = format!("http://localhost/_synapse/admin/v2/rooms/{room}");
    let (status, body) = send(
        admin(TestClient::delete(&delete_url)).json(&json!({"purge":true})),
        &service,
    )
    .await;
    assert!(status.is_server_error(), "{status}: {body}");
    assert!(!crate::room::user::is_joined(&owner, &room).await.unwrap());
    assert!(
        data::room::get_alias_room_id(&alias)
            .await
            .unwrap()
            .is_none()
    );
    let remaining = events::table
        .filter(events::room_id.eq(&room))
        .filter(events::state_key.is_null())
        .count()
        .get_result::<i64>(&mut data::db())
        .await
        .unwrap();
    assert!(remaining > 0, "failed purge rolled back its deletions");
    data::db().transaction::<_,data::DataError,_>(async |conn| {
        conn.batch_execute("DROP TRIGGER reject_history_delete ON event_datas; DROP FUNCTION reject_history_delete();").await?;
        Ok(())
    }).await.unwrap();
    let response = ok(
        admin(TestClient::delete(&delete_url)).json(&json!({"purge":false})),
        &service,
    )
    .await;
    assert_eq!(
        response["kicked_users"],
        json!([]),
        "retry must not report already departed users as newly kicked"
    );
    // Another room verifies the successful path reports the member actually removed.
    let created = ok(
        admin(TestClient::post(
            "http://localhost/_matrix/client/v3/createRoom",
        ))
        .json(&json!({"room_version":"11","preset":"public_chat"})),
        &service,
    )
    .await;
    let second = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
    let response = ok(
        admin(TestClient::delete(format!(
            "http://localhost/_synapse/admin/v2/rooms/{second}"
        )))
        .json(&json!({"purge":false})),
        &service,
    )
    .await;
    assert_eq!(response["kicked_users"], json!([owner.as_str()]));
    assert!(!crate::room::user::is_joined(&owner, &second).await.unwrap());
    audit_contract_extensions(&service, &owner).await;
}

async fn audit_contract_extensions(service: &Service, owner: &crate::core::UserId) {
    fn segment(value: &str) -> String {
        url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
    }
    let caps = ok(
        admin(TestClient::get(
            "http://localhost/_palpo/admin/v1/capabilities",
        )),
        service,
    )
    .await;
    for name in [
        "room_aliases",
        "room_members_pagination",
        "alias_delete_preserves_state",
        "make_room_admin",
        "report_event_details",
        "user_suspended",
        "destination_search",
        "media_keep_profiles",
    ] {
        assert_eq!(caps["capabilities"][name], true, "{name}");
    }
    let member = UserId::parse("@member:admin-contract.example").unwrap();
    data::user::create_user(&data::user::NewDbUser {
        id: member.clone(),
        ty: None,
        is_admin: false,
        is_guest: false,
        is_local: true,
        localpart: "member".into(),
        server_name: member.server_name().to_owned(),
        appservice_id: None,
        created_at: UnixMillis::now(),
    })
    .await
    .unwrap();
    data::user::ensure_profile_exists(&member).await.unwrap();
    data::user::device::create_device(
        &member,
        &OwnedDeviceId::from("D"),
        "member-token",
        None,
        None,
    )
    .await
    .unwrap();
    let (status, _) = send(
        TestClient::get("http://localhost/_palpo/admin/v1/capabilities").add_header(
            "Authorization",
            "Bearer member-token",
            true,
        ),
        service,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let created = ok(
        admin(TestClient::post(
            "http://localhost/_matrix/client/v3/createRoom",
        ))
        .json(&json!({"room_version":"11","preset":"public_chat","room_alias_name":"main-audit"})),
        service,
    )
    .await;
    let room = RoomId::parse(created["room_id"].as_str().unwrap()).unwrap();
    let rid = segment(room.as_str());
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{rid}/join"
        ))
        .add_header("Authorization", "Bearer member-token", true)
        .json(&json!({})),
        service,
    )
    .await;
    for alias in ["#a:admin-contract.example", "#b:admin-contract.example"] {
        ok(
            admin(TestClient::put(format!(
                "http://localhost/_matrix/client/v3/directory/room/{}",
                segment(alias)
            )))
            .json(&json!({"room_id":room})),
            service,
        )
        .await;
    }
    let aliases = ok(
        admin(TestClient::get(format!(
            "http://localhost/_palpo/admin/v1/rooms/{rid}/aliases"
        ))),
        service,
    )
    .await;
    assert_eq!(aliases["aliases"].as_array().unwrap().len(), 3);
    let canonical_url =
        format!("http://localhost/_matrix/client/v3/rooms/{rid}/state/m.room.canonical_alias");
    ok(admin(TestClient::put(&canonical_url)).json(&json!({"alias":"#main-audit:admin-contract.example","alt_aliases":["#a:admin-contract.example","#b:admin-contract.example"],"org.example.custom":true})),service).await;
    ok(
        admin(TestClient::delete(format!(
            "http://localhost/_matrix/client/v3/directory/room/{}",
            segment("#a:admin-contract.example")
        ))),
        service,
    )
    .await;
    let canonical = ok(admin(TestClient::get(&canonical_url)), service).await;
    assert_eq!(canonical["alias"], "#main-audit:admin-contract.example");
    assert_eq!(
        canonical["alt_aliases"],
        json!(["#b:admin-contract.example"])
    );
    assert_eq!(canonical["org.example.custom"], true);
    let first = ok(
        admin(TestClient::get(format!(
            "http://localhost/_palpo/admin/v1/rooms/{rid}/members?from=0&limit=1"
        ))),
        service,
    )
    .await;
    let second = ok(
        admin(TestClient::get(format!(
            "http://localhost/_palpo/admin/v1/rooms/{rid}/members?from=1&limit=1"
        ))),
        service,
    )
    .await;
    assert_eq!(first["total"], 2);
    assert_eq!(first["members"].as_array().unwrap().len(), 1);
    assert_ne!(first["members"], second["members"]);
    ok(
        admin(TestClient::post(format!(
            "http://localhost/_palpo/admin/v1/rooms/{rid}/make_room_admin"
        )))
        .json(&json!({"user_id":member})),
        service,
    )
    .await;
    let powers = ok(
        admin(TestClient::get(format!(
            "http://localhost/_matrix/client/v3/rooms/{rid}/state/m.room.power_levels"
        ))),
        service,
    )
    .await;
    assert_eq!(powers["users"][member.as_str()], 100);
    let sent = ok(
        TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{rid}/send/m.room.message/reported"
        ))
        .add_header("Authorization", "Bearer member-token", true)
        .json(&json!({"msgtype":"m.text","body":"actual reported event"})),
        service,
    )
    .await;
    let report = data::room::create_event_report(data::room::NewDbEventReport::new(
        room.clone(),
        crate::core::EventId::parse(sent["event_id"].as_str().unwrap()).unwrap(),
        owner.to_owned(),
        Some("reason".into()),
        Some(json!({"reason":"reason","sender":"@spoof:example"})),
        Some(-10),
    ))
    .await
    .unwrap();
    for request in [
        admin(TestClient::get(format!(
            "http://localhost/_palpo/admin/v1/event_reports/{report}"
        ))),
        admin(TestClient::put(format!(
            "http://localhost/_palpo/admin/v1/event_reports/{report}"
        )))
        .json(&json!({"status":"resolved"})),
    ] {
        let detail = ok(request, service).await;
        assert_eq!(detail["sender"], member.as_str());
        assert_eq!(detail["user_id"], owner.as_str());
        assert_eq!(
            detail["event_json"]["content"]["body"],
            "actual reported event"
        );
    }
    // A signature added after initial hash/sign must be counted before persistence.
    let (pdu, mut signed) = crate::event::PduBuilder {
        content: serde_json::value::to_raw_value(
            &json!({"msgtype":"m.text","body":"a".repeat(63_000)}),
        )
        .unwrap(),
        ..Default::default()
    }
    .hash_sign(owner, &room, &crate::core::RoomVersionId::V11)
    .await
    .unwrap();
    let extra = (0..40)
        .map(|n| {
            (
                format!("ed25519:k{n}"),
                crate::core::serde::CanonicalJsonValue::String("A".repeat(86)),
            )
        })
        .collect();
    signed
        .get_mut("signatures")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "extra.example".into(),
            crate::core::serde::CanonicalJsonValue::Object(extra),
        );
    let id = pdu.event_id.clone();
    assert!(
        crate::event::PduBuilder::save_as_outlier(pdu, signed, owner)
            .await
            .is_err()
    );
    assert_eq!(
        events::table
            .filter(events::id.eq(id))
            .count()
            .get_result::<i64>(&mut data::db())
            .await
            .unwrap(),
        0
    );
    for suspended in [true, false] {
        ok(
            admin(TestClient::put(format!(
                "http://localhost/_palpo/admin/v1/suspend/{}",
                segment(member.as_str())
            )))
            .json(&json!({"suspend":suspended})),
            service,
        )
        .await;
        let user = ok(
            admin(TestClient::get(format!(
                "http://localhost/_palpo/admin/v2/users/{}",
                segment(member.as_str())
            ))),
            service,
        )
        .await;
        assert_eq!(user["suspended"], suspended);
    }
    let rows: Vec<_> = (0..27)
        .map(|n| data::sending::NewDbOutgoingRequest {
            kind: "normal".into(),
            appservice_id: None,
            user_id: None,
            pushkey: None,
            server_id: Some(
                crate::core::ServerName::parse(format!("destination-{n:02}.example")).unwrap(),
            ),
            pdu_id: None,
            edu_json: Some(b"{}".to_vec()),
        })
        .collect();
    diesel::insert_into(outgoing_requests::table)
        .values(rows)
        .execute(&mut data::db())
        .await
        .unwrap();
    let page=ok(admin(TestClient::get("http://localhost/_palpo/admin/v1/federation/destinations?destination=26&order_by=destination&dir=b&from=0&limit=1")),service).await;
    assert_eq!(page["total"], 1);
    assert_eq!(
        page["destinations"][0]["destination"],
        "destination-26.example"
    );
    let page=ok(admin(TestClient::get("http://localhost/_palpo/admin/v1/federation/destinations?order_by=destination&dir=b&from=0&limit=1")),service).await;
    assert_eq!(page["total"], 27);
    assert_eq!(
        page["destinations"][0]["destination"],
        "destination-26.example"
    );
    // Storage is isolated and contains no user files; missing object deletion is idempotent.
    let storage = tempfile::tempdir().unwrap();
    crate::storage::init(&crate::config::StorageConfig::Fs {
        root: storage.path().to_str().unwrap().to_owned(),
    })
    .unwrap();
    let avatar = crate::core::OwnedMxcUri::from("mxc://admin-contract.example/profile");
    data::user::set_avatar_url(owner, &avatar).await.unwrap();
    ok(
        admin(TestClient::put(format!(
            "http://localhost/_matrix/client/v3/rooms/{rid}/state/m.room.avatar"
        )))
        .json(&json!({"url":"mxc://admin-contract.example/room-avatar"})),
        service,
    )
    .await;
    for id in ["profile", "room-avatar", "unused"] {
        data::media::insert_metadata(&data::media::NewDbMetadata {
            media_id: id.into(),
            origin_server: owner.server_name().to_owned(),
            content_type: Some("image/png".into()),
            disposition_type: None,
            file_name: None,
            file_extension: None,
            file_size: 100,
            file_hash: None,
            created_by: Some(owner.to_owned()),
            created_at: UnixMillis(1),
        })
        .await
        .unwrap();
    }
    let deletion=ok(admin(TestClient::post("http://localhost/_palpo/admin/v1/media/delete?before_ts=1800000000000&size_gt=0&keep_profiles=true")),service).await;
    assert_eq!(deletion["deleted_media"], json!(["unused"]));
    assert!(
        data::media::get_metadata(owner.server_name(), "profile")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        data::media::get_metadata(owner.server_name(), "room-avatar")
            .await
            .unwrap()
            .is_some()
    );
    let deletion=ok(admin(TestClient::post("http://localhost/_palpo/admin/v1/media/delete?before_ts=1800000000000&size_gt=0&keep_profiles=false")),service).await;
    assert_eq!(deletion["total"], 2);

    // Alias ownership survives leaving a room, even if the creator retains a
    // power-level entry that would otherwise permit updating canonical aliases.
    let departed_aliases = [
        "#departed-main:admin-contract.example",
        "#departed-alt:admin-contract.example",
    ];
    for alias in departed_aliases {
        ok(
            TestClient::put(format!(
                "http://localhost/_matrix/client/v3/directory/room/{}",
                segment(alias)
            ))
            .add_header("Authorization", "Bearer member-token", true)
            .json(&json!({"room_id":room})),
            service,
        )
        .await;
    }
    let canonical_before = json!({
        "alias":departed_aliases[0],
        "alt_aliases":["#b:admin-contract.example",departed_aliases[1]],
        "org.example.custom":true
    });
    let canonical_event = ok(
        admin(TestClient::put(&canonical_url)).json(&canonical_before),
        service,
    )
    .await;
    ok(
        TestClient::post(format!(
            "http://localhost/_matrix/client/v3/rooms/{rid}/leave"
        ))
        .add_header("Authorization", "Bearer member-token", true)
        .json(&json!({})),
        service,
    )
    .await;
    assert!(!crate::room::user::is_joined(&member, &room).await.unwrap());
    assert!(
        crate::room::get_power_levels(&room)
            .await
            .unwrap()
            .user_can_send_state(
                &member,
                crate::core::events::StateEventType::RoomCanonicalAlias
            )
    );
    for alias in departed_aliases {
        ok(
            TestClient::delete(format!(
                "http://localhost/_matrix/client/v3/directory/room/{}",
                segment(alias)
            ))
            .add_header("Authorization", "Bearer member-token", true),
            service,
        )
        .await;
        assert!(
            data::room::get_alias_room_id(&RoomAliasId::parse(alias).unwrap())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            ok(admin(TestClient::get(&canonical_url)), service).await,
            canonical_before
        );
        let state = crate::room::get_state(
            &room,
            &crate::core::events::StateEventType::RoomCanonicalAlias,
            "",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            state.event_id.as_str(),
            canonical_event["event_id"].as_str().unwrap()
        );
    }
}
