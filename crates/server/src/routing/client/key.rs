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
use crate::{AuthArgs, CjsonResult, DepotExt, JsonResult, cjson_ok, data, json_ok, room};

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

/// #POST /_matrix/client/r0/keys/changes
/// Gets a list of users who have updated their device identity keys since the previous sync token.
///
/// - TODO: left users
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
    if from_tk.is_publication_page() || to_tk.is_publication_page() {
        return Err(crate::MatrixError::invalid_param(
            "message page cursor is not a sync position",
        )
        .into());
    }
    let mut device_list_updates = HashSet::new();
    device_list_updates.extend(
        data::user::keys_changed_users(sender_id, from_tk.event_sn(), Some(to_tk.event_sn()))
            .await?,
    );

    for room_id in data::user::joined_rooms(sender_id).await? {
        device_list_updates.extend(
            room::keys_changed_users(&room_id, from_tk.event_sn(), Some(to_tk.event_sn())).await?,
        );
    }
    json_ok(KeyChangesResBody {
        changed: device_list_updates.into_iter().collect(),
        left: Vec::new(), // TODO
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

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_normal_fallback_upload_claim_and_sync_lifecycle() {
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
                .push(authed_router()),
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
        let idle_args = serde_json::from_value(json!({"since":sync.next_batch})).unwrap();
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
    }
}
