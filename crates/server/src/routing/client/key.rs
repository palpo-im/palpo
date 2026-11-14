mod device_signing;
mod signature;

use std::collections::HashSet;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::key::{
    ClaimKeysReqBody, ClaimKeysResBody, KeyChangesReqArgs, KeyChangesResBody, KeysReqBody,
    KeysResBody, UploadKeysReqBody, UploadKeysResBody,
};
use crate::event::BatchToken;
use crate::user::key;
use crate::{AuthArgs, CjsonResult, DepotExt, JsonResult, cjson_ok, data, json_ok};

pub fn authed_router() -> Router {
    Router::with_path("keys")
        .push(Router::with_path("claim").post(claim_keys))
        .push(Router::with_path("query").post(query_keys))
        .push(Router::with_path("upload").post(upload_keys))
        .push(Router::with_path("changes").get(get_key_changes))
        .push(Router::with_path("signatures/upload").post(signature::upload))
        .push(Router::with_path("device_signing/upload").post(device_signing::upload))
}

/// #POST /_matrix/client/r0/keys/claim
/// Claims one-time keys
#[endpoint]
async fn claim_keys(
    _aa: AuthArgs,
    body: JsonBody<ClaimKeysReqBody>,
) -> CjsonResult<ClaimKeysResBody> {
    cjson_ok(key::claim_one_time_keys(&body.one_time_keys).await?)
}

/// #POST /_matrix/client/r0/keys/query
/// Get end-to-end encryption keys for the given users.
///
/// - Always fetches users from other servers over federation
/// - Gets master keys, self-signing keys, user signing keys and device keys.
/// - The master and self-signing keys contain signatures that the user is allowed to see
#[endpoint]
async fn query_keys(
    _aa: AuthArgs,
    body: JsonBody<KeysReqBody>,
    depot: &mut Depot,
) -> CjsonResult<KeysResBody> {
    let authed = depot.authed_info()?;
    cjson_ok(
        key::query_keys(
            Some(authed.user_id()),
            &body.device_keys,
            |u| u == authed.user_id(),
            false,
        )
        .await?,
    )
}

/// #POST /_matrix/client/r0/keys/upload
/// Publish end-to-end encryption keys for the sender device.
///
/// - Adds one time keys
/// - If there are no device keys yet: Adds device keys (TODO: merge with existing keys?)
#[endpoint]
async fn upload_keys(
    _aa: AuthArgs,
    body: JsonBody<UploadKeysReqBody>,
    depot: &mut Depot,
) -> JsonResult<UploadKeysResBody> {
    let authed = depot.authed_info()?;

    let mut fallback_algorithms = HashSet::new();
    for key_id in body.fallback_keys.keys() {
        if !fallback_algorithms.insert(key_id.algorithm().to_string()) {
            return Err(crate::MatrixError::invalid_param(
                "Only one fallback key per algorithm may be uploaded.",
            )
            .into());
        }
    }

    for (key_id, one_time_key) in &body.one_time_keys {
        crate::user::add_one_time_key(
            authed.user_id(),
            authed.require_device_id()?,
            key_id,
            one_time_key,
        )
        .await?;
    }

    if let Some(device_keys) = &body.device_keys {
        crate::user::add_device_keys(authed.user_id(), authed.require_device_id()?, device_keys)
            .await?;
    }

    for (key_id, fallback_key) in &body.fallback_keys {
        crate::user::add_fallback_key(
            authed.user_id(),
            authed.require_device_id()?,
            key_id,
            fallback_key,
        )
        .await?;
    }

    json_ok(UploadKeysResBody {
        one_time_key_counts: data::user::count_one_time_keys(
            authed.user_id(),
            authed.require_device_id()?,
        )
        .await?,
    })
}

/// #GET /_matrix/client/v3/keys/changes
/// Gets a list of users who have updated their device identity keys since the previous sync token.
#[endpoint]
async fn get_key_changes(
    _aa: AuthArgs,
    args: KeyChangesReqArgs,
    depot: &mut Depot,
) -> JsonResult<KeyChangesResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();

    let from_tk: BatchToken = args.from.parse()?;
    let to_tk: BatchToken = args.to.parse()?;
    if matches!(
        from_tk,
        BatchToken::PublicationPage { .. } | BatchToken::Historic { .. }
    ) || matches!(
        to_tk,
        BatchToken::PublicationPage { .. } | BatchToken::Historic { .. }
    ) {
        return Err(crate::MatrixError::invalid_param(
            "message page cursor is not a sync position",
        )
        .into());
    }
    let lists = crate::user::device_lists::changes(
        sender_id,
        from_tk.stream_ordering(),
        to_tk.stream_ordering(),
    )
    .await?;
    json_ok(KeyChangesResBody {
        changed: lists.changed,
        left: lists.left,
    })
}

#[cfg(test)]
mod fallback_tests {
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;

    async fn post(service: &Service, path: &str, body: Value) -> (Option<StatusCode>, Value) {
        let mut response = TestClient::post(format!("http://localhost/keys/{path}"))
            .add_header("authorization", "Bearer fallback-upload", true)
            .json(&body)
            .send(service)
            .await;
        (response.status_code, response.take_json().await.unwrap())
    }

    #[test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    fn database_normal_fallback_upload_claim_and_sync_lifecycle() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(normal_fallback_upload_claim_and_sync_lifecycle());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    async fn normal_fallback_upload_claim_and_sync_lifecycle() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name":"fallback.example", "db":{"url":"unused-test-config"}
            }))
            .unwrap()
        });
        let user =
            crate::core::UserId::parse(format!("@fallbackupload:{}", crate::config::server_name()))
                .unwrap();
        crate::data::user::create_user(&crate::data::user::NewDbUser {
            id: user.clone(),
            ty: None,
            is_admin: false,
            is_guest: false,
            is_local: true,
            localpart: user.localpart().into(),
            server_name: user.server_name().to_owned(),
            appservice_id: None,
            created_at: crate::core::UnixMillis::now(),
        })
        .await
        .unwrap();
        let device: crate::core::OwnedDeviceId = "FALLBACK_UPLOAD".into();
        crate::data::user::device::create_device(&user, &device, "fallback-upload", None, None)
            .await
            .unwrap();
        let service = Service::new(
            Router::new()
                .hoop(crate::hoops::auth_by_access_token)
                .push(authed_router())
                .push(
                    Router::with_path("sync").get(crate::routing::client::sync_v3::sync_events_v3),
                ),
        );
        let upload = json!({"fallback_keys":{"signed_curve25519:first":{"key":"ZmFsbGJhY2s","fallback":true,"signatures":{}}},
            "one_time_keys":{"signed_curve25519:once":"b25jZQ"}});
        assert_eq!(
            post(&service, "upload", upload.clone()).await.0,
            Some(StatusCode::OK)
        );
        let args = serde_json::from_value(json!({})).unwrap();
        let sync = crate::sync_v3::sync_events(&user, &device, &args)
            .await
            .unwrap();
        assert_eq!(
            sync.device_unused_fallback_key_types,
            Some(vec![crate::core::DeviceKeyAlgorithm::SignedCurve25519])
        );
        assert_eq!(
            sync.device_one_time_keys_count
                .get(&crate::core::DeviceKeyAlgorithm::SignedCurve25519),
            Some(&1)
        );
        // Force the idle fast path under both MSC4494 configurations. The
        // response must preserve this cursor while still reporting key state.
        let idle_since = format!("p{}", data::curr_sn().await.unwrap() + 100);
        let idle_args = serde_json::from_value(json!({"since":idle_since})).unwrap();
        let mut response = TestClient::get(format!(
            "http://localhost/sync?since={idle_since}&timeout=0"
        ))
        .add_header("authorization", "Bearer fallback-upload", true)
        .send(&service)
        .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let body: Value = response.take_json().await.unwrap();
        assert_eq!(body["next_batch"], idle_since);
        assert_eq!(body["device_one_time_keys_count"]["signed_curve25519"], 1);

        let stream_before_claim = data::curr_sn().await.unwrap();
        let claim = json!({"one_time_keys":{user.as_str():{device.as_str():"signed_curve25519"}}});
        let (status, keys) = post(&service, "claim", claim.clone()).await;
        assert_eq!(status, Some(StatusCode::OK), "{keys}");
        assert!(
            keys["one_time_keys"][user.as_str()][device.as_str()]
                .get("signed_curve25519:once")
                .is_some()
        );
        for _ in 0..2 {
            let (status, keys) = post(&service, "claim", claim.clone()).await;
            assert_eq!(status, Some(StatusCode::OK), "{keys}");
            assert!(
                keys["one_time_keys"][user.as_str()][device.as_str()]
                    .get("signed_curve25519:first")
                    .is_some()
            );
        }
        assert_eq!(data::curr_sn().await.unwrap(), stream_before_claim);
        let idle_sync = crate::sync_v3::sync_events(&user, &device, &idle_args)
            .await
            .unwrap();
        assert!(
            idle_sync.device_one_time_keys_count.is_empty(),
            "claimed one-time keys must disappear from idle sync counts"
        );
        assert_eq!(
            idle_sync.device_unused_fallback_key_types,
            Some(vec![]),
            "a claimed fallback key must be reported on an idle incremental sync"
        );
        let sync = crate::sync_v3::sync_events(&user, &device, &args)
            .await
            .unwrap();
        assert_eq!(sync.device_unused_fallback_key_types, Some(vec![]));
        let retry = json!({"fallback_keys":upload["fallback_keys"]});
        assert_eq!(
            post(&service, "upload", retry).await.0,
            Some(StatusCode::OK)
        );
        assert!(
            data::user::key::unused_fallback_key_types(&user, &device)
                .await
                .unwrap()
                .is_empty()
        );
        let changed_material = json!({"fallback_keys":{"signed_curve25519:first":{"key":"ZGlmZmVyZW50","fallback":true,"signatures":{}}}});
        let (status, rejected) = post(&service, "upload", changed_material).await;
        assert_eq!(status, Some(StatusCode::BAD_REQUEST), "{rejected}");
        let (_, retained) = post(&service, "claim", claim.clone()).await;
        assert_eq!(
            retained["one_time_keys"][user.as_str()][device.as_str()]["signed_curve25519:first"]["key"],
            "ZmFsbGJhY2s"
        );
        assert!(
            data::user::key::unused_fallback_key_types(&user, &device)
                .await
                .unwrap()
                .is_empty()
        );
        let replacement = json!({"fallback_keys":{"signed_curve25519:second":{"key":"bmV3","fallback":true,"signatures":{}}}});
        let (first, second) = tokio::join!(
            post(&service, "upload", replacement.clone()),
            post(&service, "upload", replacement)
        );
        assert_eq!(first.0, Some(StatusCode::OK), "{:?}", first.1);
        assert_eq!(second.0, Some(StatusCode::OK), "{:?}", second.1);
        assert_eq!(
            data::user::key::unused_fallback_key_types(&user, &device)
                .await
                .unwrap(),
            vec![crate::core::DeviceKeyAlgorithm::SignedCurve25519]
        );
        let (_, keys) = post(&service, "claim", claim).await;
        assert!(
            keys["one_time_keys"][user.as_str()][device.as_str()]
                .get("signed_curve25519:second")
                .is_some()
        );
        let invalid =
            json!({"fallback_keys":{"signed_curve25519:a":"YQ", "signed_curve25519:b":"Yg"}});
        let (status, _) = post(&service, "upload", invalid).await;
        assert_eq!(status, Some(StatusCode::BAD_REQUEST));
        // Uploading keys need not publish a room event: reusing the same idle
        // cursor must report newly available keys, scoped to this device.
        assert_eq!(
            post(
                &service,
                "upload",
                json!({
                    "one_time_keys":{"signed_curve25519:replenished":"bmV3"}
                })
            )
            .await
            .0,
            Some(StatusCode::OK)
        );
        let replenished = crate::sync_v3::sync_events(&user, &device, &idle_args)
            .await
            .unwrap();
        assert_eq!(
            replenished
                .device_one_time_keys_count
                .get(&crate::core::DeviceKeyAlgorithm::SignedCurve25519),
            Some(&1)
        );
        let other: crate::core::OwnedDeviceId = "OTHER_DEVICE".into();
        crate::data::user::device::create_device(&user, &other, "other-upload", None, None)
            .await
            .unwrap();
        let other_sync = crate::sync_v3::sync_events(&user, &other, &idle_args)
            .await
            .unwrap();
        assert!(other_sync.device_one_time_keys_count.is_empty());
        // Storage failure must not look like a successful zero-key response.
        use horme::RunQueryDsl;
        diesel::sql_query("ALTER TABLE e2e_one_time_keys RENAME TO review_unavailable_otks")
            .execute(&mut crate::data::db())
            .await
            .unwrap();
        let failed_idle = crate::sync_v3::sync_events(&user, &device, &idle_args).await;
        let failed_full = crate::sync_v3::sync_events(&user, &device, &args).await;
        diesel::sql_query("ALTER TABLE review_unavailable_otks RENAME TO e2e_one_time_keys")
            .execute(&mut crate::data::db())
            .await
            .unwrap();
        assert!(failed_idle.is_err());
        assert!(failed_full.is_err());
        assert!(
            crate::sync_v3::sync_events(&user, &device, &idle_args)
                .await
                .is_ok()
        );
    }
}

#[cfg(test)]
#[path = "key_changes_tests.rs"]
mod key_changes_tests;
