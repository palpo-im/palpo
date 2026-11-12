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
}
