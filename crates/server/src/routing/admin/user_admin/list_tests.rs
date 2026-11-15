//! Admin list contract and pagination through real routes and PostgreSQL.
use std::collections::BTreeSet;

use diesel::prelude::*;
use horme::RunQueryDsl;
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use serde_json::{Value, json};

use crate::core::{OwnedDeviceId, OwnedUserId, UnixMillis};
use crate::{config, data};

const TOKEN: &str = "admin-list-review-token";

async fn get(service: &Service, version: u8, query: &str) -> (StatusCode, Value) {
    let router = service.router();
    let builder = TestClient::get(format!(
        "http://localhost/_synapse/admin/v{version}/users?{query}"
    ))
    .add_header("Authorization", format!("Bearer {TOKEN}"), true);
    let mut response = tokio::spawn(async move { builder.send(router).await })
        .await
        .unwrap();
    let status = response.status_code.unwrap();
    (status, response.take_json().await.unwrap())
}

fn account(localpart: &str, admin: bool) -> data::user::NewDbUser {
    data::user::NewDbUser {
        id: format!("@{localpart}:admin-list.test").try_into().unwrap(),
        ty: None,
        is_admin: admin,
        is_guest: false,
        is_local: true,
        localpart: localpart.into(),
        server_name: "admin-list.test".try_into().unwrap(),
        appservice_id: None,
        created_at: UnixMillis(1_700_000_000_000),
    }
}

#[tokio::test]
#[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
async fn review_admin_list_deactivation_contract_and_clamped_pagination() {
    crate::test_database::init();
    config::CONFIG
        .set(
            serde_json::from_value(json!({
                "server_name":"admin-list.test","db":{"url":"unused-test-config"}
            }))
            .unwrap(),
        )
        .unwrap();
    let admin: OwnedUserId = "@admin:admin-list.test".try_into().unwrap();
    data::user::create_user(&account("admin", true))
        .await
        .unwrap();
    data::user::device::create_device(&admin, &OwnedDeviceId::from("D"), TOKEN, None, None)
        .await
        .unwrap();
    for localpart in ["filteractive", "filterinactive"] {
        data::user::create_user(&account(localpart, false))
            .await
            .unwrap();
    }
    diesel::update(data::schema::users::table.find("@filterinactive:admin-list.test"))
        .set(data::schema::users::deactivated_at.eq(Some(UnixMillis::now())))
        .execute(&mut data::db())
        .await
        .unwrap();
    let service = Service::new(crate::routing::root());
    for (version, flag, expected) in [
        (2, "", vec![false]),
        (2, "&deactivated=false", vec![false]),
        (2, "&deactivated=true", vec![false, true]),
        (3, "", vec![false, true]),
        (3, "&deactivated=false", vec![false]),
        (3, "&deactivated=true", vec![true]),
        (3, "&not_deactivated=true", vec![false]),
        (3, "&deactivated=true&not_deactivated=true", vec![true]),
    ] {
        let (status, body) = get(&service, version, &format!("name=filter{flag}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut states = body["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|user| user["deactivated"].as_bool().unwrap())
            .collect::<Vec<_>>();
        states.sort();
        assert_eq!(states, expected, "v{version} {flag}");
        assert_eq!(body["total"].as_u64().unwrap(), states.len() as u64);
        assert!(body.get("next_token").is_none());
    }
    let users = (0..1500)
        .map(|i| account(&format!("page{i:04}"), false))
        .collect::<Vec<_>>();
    for chunk in users.chunks(500) {
        diesel::insert_into(data::schema::users::table)
            .values(chunk)
            .execute(&mut data::db())
            .await
            .unwrap();
    }
    for version in [2, 3] {
        let mut seen = BTreeSet::new();
        let mut from = "0".to_owned();
        loop {
            let (status, page) = get(
                &service,
                version,
                &format!("name=page&order_by=creation_ts&limit=1001&from={from}"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{page}");
            assert_eq!(page["total"], 1500);
            let rows = page["users"].as_array().unwrap();
            assert!(!rows.is_empty());
            assert!(rows.len() <= 1000);
            for user in rows {
                assert!(
                    seen.insert(user["name"].as_str().unwrap().to_owned()),
                    "page ordering must not duplicate accounts"
                );
            }
            if let Some(next) = page["next_token"].as_str() {
                assert_eq!(next, "1000", "the cap must not hide the next page");
                from = next.into();
            } else {
                break;
            }
        }
        assert_eq!(seen.len(), 1500);
        let (_, last) = get(&service, version, "name=page&limit=500&from=1000").await;
        assert_eq!(last["users"].as_array().unwrap().len(), 500);
        assert!(
            last.get("next_token").is_none(),
            "exact last pages have no continuation"
        );
        let (_, empty) = get(&service, version, "name=page&limit=2000&from=1500").await;
        assert!(empty["users"].as_array().unwrap().is_empty());
        assert!(empty.get("next_token").is_none());
        for query in ["limit=0", "limit=-1", "from=-1"] {
            let (status, error) = get(&service, version, query).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {error}");
        }
    }
}
