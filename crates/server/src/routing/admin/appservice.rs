use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::appservice::{Namespaces, Registration};
use crate::{EmptyResult, JsonResult, MatrixError, appservice as svc, empty_ok, json_ok};

/// Request/response body mirroring [`Registration`] but deriving `ToSchema`
/// so it can be used with the OpenAPI-aware extractors.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppserviceRegistrationBody {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
    pub as_token: String,
    pub hs_token: String,
    pub sender_localpart: String,
    pub namespaces: serde_json::Value,
    #[serde(default)]
    pub rate_limited: Option<bool>,
    #[serde(default)]
    pub protocols: Option<Vec<String>>,
    #[serde(default)]
    pub receive_ephemeral: bool,
    #[serde(default, rename = "io.element.msc4190")]
    pub device_management: bool,
    #[serde(default)]
    pub disabled: bool,
}

impl AppserviceRegistrationBody {
    fn into_registration(self) -> Result<Registration, serde_json::Error> {
        let namespaces: Namespaces = serde_json::from_value(self.namespaces)?;
        Ok(Registration {
            id: self.id,
            url: self.url,
            as_token: self.as_token,
            hs_token: self.hs_token,
            sender_localpart: self.sender_localpart,
            namespaces,
            rate_limited: self.rate_limited,
            protocols: self.protocols,
            receive_ephemeral: self.receive_ephemeral,
            device_management: self.device_management,
        })
    }

    fn from_registration(r: Registration, disabled: bool) -> Self {
        Self {
            id: r.id,
            url: r.url,
            as_token: r.as_token,
            hs_token: r.hs_token,
            sender_localpart: r.sender_localpart,
            namespaces: serde_json::to_value(r.namespaces).unwrap_or_default(),
            rate_limited: r.rate_limited,
            protocols: r.protocols,
            receive_ephemeral: r.receive_ephemeral,
            device_management: r.device_management,
            disabled,
        }
    }
}

/// Summary of a registered appservice (without secret tokens).
#[derive(Debug, Serialize, ToSchema)]
pub struct AppserviceSummary {
    pub id: String,
    pub url: Option<String>,
    pub sender_localpart: String,
    pub disabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListAppservicesResponse {
    pub appservices: Vec<AppserviceSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AppserviceResponse {
    pub id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAppserviceUrlBody {
    pub url: String,
    // Required even when null: omission must never become a blind update.
    #[serde(deserialize_with = "Option::<String>::deserialize")]
    #[salvo(schema(required))]
    pub expected_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateAppserviceUrlResponse {
    pub id: String,
    pub url: String,
}

pub fn router() -> Router {
    Router::with_path("v1").push(
        Router::with_path("appservices")
            .get(list_appservices)
            .post(register_appservice)
            .push(
                Router::with_path("{id}")
                    .get(get_appservice)
                    .delete(delete_appservice)
                    .push(Router::with_path("url").put(update_appservice_url))
                    .push(Router::with_path("disable").post(disable_appservice))
                    .push(Router::with_path("enable").post(enable_appservice)),
            ),
    )
}

/// PUT /_palpo/admin/v1/appservices/{id}/url
///
/// Compare the exact stored URL, then update only that column. A null
/// expected_url matches a registration which has no callback URL yet.
/// Stale expectations return 409, including retries with the old URL; callers
/// must read back the registration before retrying with a new expectation.
#[endpoint(operation_id = "update_appservice_url")]
pub async fn update_appservice_url(
    id: PathParam<String>,
    body: JsonBody<UpdateAppserviceUrlBody>,
) -> JsonResult<UpdateAppserviceUrlResponse> {
    let id = id.into_inner();
    let body = body.into_inner();
    match svc::update_appservice_url(&id, body.expected_url.as_deref(), &body.url).await? {
        crate::data::appservice::UpdateUrlResult::Updated => {
            json_ok(UpdateAppserviceUrlResponse { id, url: body.url })
        }
        crate::data::appservice::UpdateUrlResult::NotFound => {
            Err(MatrixError::not_found(format!("No such appservice: {id}")).into())
        }
        crate::data::appservice::UpdateUrlResult::Mismatch => {
            let mut error =
                MatrixError::invalid_param("Appservice URL changed; read back before retrying");
            error.status_code = Some(StatusCode::CONFLICT);
            Err(error.into())
        }
    }
}

/// List all registered appservices (including disabled ones).
///
/// GET /_synapse/admin/v1/appservices
#[endpoint(operation_id = "list_appservices")]
pub async fn list_appservices() -> JsonResult<ListAppservicesResponse> {
    let rows = svc::list_all_registrations().await?;
    let appservices = rows
        .into_iter()
        .map(|(r, disabled)| AppserviceSummary {
            id: r.id,
            url: r.url,
            sender_localpart: r.sender_localpart,
            disabled,
        })
        .collect();
    json_ok(ListAppservicesResponse { appservices })
}

/// Get a single appservice registration by id.
///
/// GET /_synapse/admin/v1/appservices/{id}
#[endpoint(operation_id = "get_appservice")]
pub async fn get_appservice(id: PathParam<String>) -> JsonResult<AppserviceRegistrationBody> {
    let id = id.into_inner();
    let registration = svc::get_registration(&id)
        .await?
        .ok_or_else(|| MatrixError::not_found(format!("No such appservice: {}", id)))?;
    let disabled = svc::list_all_registrations()
        .await?
        .into_iter()
        .find(|(r, _)| r.id == id)
        .map(|(_, d)| d)
        .unwrap_or(false);
    json_ok(AppserviceRegistrationBody::from_registration(
        registration,
        disabled,
    ))
}

/// Register a new appservice.
///
/// POST /_synapse/admin/v1/appservices
///
/// Request body: a full appservice `Registration` object.
#[endpoint(operation_id = "register_appservice")]
pub async fn register_appservice(
    body: JsonBody<AppserviceRegistrationBody>,
) -> JsonResult<AppserviceResponse> {
    let body = body.into_inner();
    if body.id.is_empty() {
        return Err(MatrixError::invalid_param("id must not be empty").into());
    }
    if svc::get_registration(&body.id).await?.is_some() {
        return Err(MatrixError::invalid_param(format!(
            "Appservice with id {} already exists",
            body.id
        ))
        .into());
    }
    let registration = body
        .into_registration()
        .map_err(|e| MatrixError::invalid_param(format!("invalid namespaces: {e}")))?;
    let id = svc::register_appservice(registration).await?;
    json_ok(AppserviceResponse { id })
}

/// Delete (unregister) an appservice.
///
/// DELETE /_synapse/admin/v1/appservices/{id}
#[endpoint(operation_id = "delete_appservice")]
pub async fn delete_appservice(id: PathParam<String>) -> EmptyResult {
    let id = id.into_inner();
    if svc::get_registration(&id).await?.is_none() {
        return Err(MatrixError::not_found(format!("No such appservice: {}", id)).into());
    }
    svc::unregister_appservice(&id).await?;
    empty_ok()
}

/// Disable an appservice. Disabled appservices cannot authenticate or
/// receive events until re-enabled.
///
/// POST /_synapse/admin/v1/appservices/{id}/disable
#[endpoint(operation_id = "disable_appservice")]
pub async fn disable_appservice(id: PathParam<String>) -> EmptyResult {
    let id = id.into_inner();
    if !svc::set_appservice_disabled(&id, true).await? {
        return Err(MatrixError::not_found(format!("No such appservice: {}", id)).into());
    }
    empty_ok()
}

/// Re-enable a previously disabled appservice.
///
/// POST /_synapse/admin/v1/appservices/{id}/enable
#[endpoint(operation_id = "enable_appservice")]
pub async fn enable_appservice(id: PathParam<String>) -> EmptyResult {
    let id = id.into_inner();
    if !svc::set_appservice_disabled(&id, false).await? {
        return Err(MatrixError::not_found(format!("No such appservice: {}", id)).into());
    }
    empty_ok()
}

#[cfg(test)]
mod tests {
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;
    use crate::core::UnixMillis;
    use crate::core::identifiers::*;
    use crate::data;

    #[test]
    fn appservice_url_cas_requires_explicit_expectation_and_rejects_extra_mutations() {
        let body: UpdateAppserviceUrlBody = serde_json::from_value(json!({
            "url": "https://relay.example/fleet", "expected_url": null,
        }))
        .unwrap();
        assert!(body.expected_url.is_none());
        for body in [
            json!({"url": "https://relay.example/fleet"}),
            json!({"url": null, "expected_url": null}),
            json!({"url": "https://relay.example/fleet", "expected_url": 1}),
            json!({"url": "https://relay.example/fleet", "expected_url": null, "as_token": "replacement"}),
            json!({"url": "https://relay.example/fleet", "expected_url": null, "disabled": false}),
        ] {
            assert!(serde_json::from_value::<UpdateAppserviceUrlBody>(body).is_err());
        }
    }

    async fn put_url(service: &Service, id: &str, token: Option<&str>, body: Value) -> Response {
        let mut request = TestClient::put(format!(
            "http://localhost/_palpo/admin/v1/appservices/{id}/url"
        ))
        .json(&body);
        if let Some(token) = token {
            request = request.add_header("authorization", format!("Bearer {token}"), true);
        }
        request.send(service).await
    }

    async fn fixture_user(
        localpart: &str,
        admin: bool,
        appservice_id: Option<&str>,
        token: &str,
    ) -> OwnedUserId {
        let id = UserId::parse(format!("@{localpart}:url-cas.example")).unwrap();
        data::user::create_user(&data::user::NewDbUser {
            id: id.clone(),
            ty: None,
            is_admin: admin,
            is_guest: false,
            is_local: true,
            localpart: localpart.to_owned(),
            server_name: id.server_name().to_owned(),
            appservice_id: appservice_id.map(str::to_owned),
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        let device_id: OwnedDeviceId = "URL_CAS_DEVICE".into();
        data::user::device::create_device(&id, &device_id, token, None, None)
            .await
            .unwrap();
        id
    }

    async fn stored_registration(id: &str) -> Value {
        serde_json::to_value(
            data::appservice::find_registration(id)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_appservice_url_cas_admin_and_concurrent_identity_preservation() {
        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "url-cas.example", "db": {"url": "unused-test-config"},
            }))
            .unwrap()
        });
        // Use the production router, authentication and admin hoop with real
        // persisted tokens, rather than injecting a pre-authorized depot.
        let service = Service::new(crate::routing::admin::router());
        fixture_user("admin", true, None, "cas-admin-token").await;
        fixture_user("member", false, None, "cas-member-token").await;
        let registration: Registration = serde_json::from_value(json!({
            "id": "url-cas-fixture", "url": "http://original.example/old-prefix",
            "as_token": "cas-original-as-token", "hs_token": "cas-original-hs-token",
            "sender_localpart": "fleet_sender",
            "namespaces": {"users": [{"exclusive": true, "regex": "^@fleet_.*:url-cas\\.example$"}], "aliases": [], "rooms": []},
            "rate_limited": true, "protocols": ["fixture"], "receive_ephemeral": true,
            "io.element.msc4190": true,
        })).unwrap();
        svc::register_appservice(registration).await.unwrap();
        svc::set_appservice_disabled("url-cas-fixture", true)
            .await
            .unwrap();
        let virtual_user = fixture_user(
            "fleet_child",
            false,
            Some("url-cas-fixture"),
            "cas-child-token",
        )
        .await;
        let child_before =
            data::user::authenticate_token("cas-child-token", std::time::Duration::ZERO)
                .await
                .unwrap()
                .unwrap();
        let before = stored_registration("url-cas-fixture").await;
        let target = "https://relay.example/palpo-side/fleet-1";
        let payload = json!({"url": target, "expected_url": before["url"]});

        for (token, expected_status) in [
            (None, StatusCode::UNAUTHORIZED),
            (Some("cas-invalid-token"), StatusCode::UNAUTHORIZED),
            (Some("cas-member-token"), StatusCode::FORBIDDEN),
            (Some("cas-child-token"), StatusCode::FORBIDDEN),
        ] {
            let response = put_url(&service, "url-cas-fixture", token, payload.clone()).await;
            assert_eq!(response.status_code, Some(expected_status));
        }
        for invalid in [
            json!({"url": target}),
            json!({"url": "https://user:secret@relay.example/path", "expected_url": before["url"]}),
            json!({"url": "https://relay.example/path?token=secret", "expected_url": before["url"]}),
            json!({"url": target, "expected_url": before["url"], "disabled": false}),
        ] {
            let response = put_url(
                &service,
                "url-cas-fixture",
                Some("cas-admin-token"),
                invalid,
            )
            .await;
            assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
        }
        assert_eq!(stored_registration("url-cas-fixture").await, before);
        let missing = put_url(
            &service,
            "missing",
            Some("cas-admin-token"),
            payload.clone(),
        )
        .await;
        assert_eq!(missing.status_code, Some(StatusCode::NOT_FOUND));

        let mut updated = put_url(
            &service,
            "url-cas-fixture",
            Some("cas-admin-token"),
            payload.clone(),
        )
        .await;
        assert_eq!(updated.status_code, Some(StatusCode::OK));
        assert_eq!(
            updated.take_json::<Value>().await.unwrap(),
            json!({"id": "url-cas-fixture", "url": target})
        );
        let mut expected = before.clone();
        expected["url"] = json!(target);
        assert_eq!(stored_registration("url-cas-fixture").await, expected);

        // A lost-response retry cannot bypass CAS merely because desired==current.
        let stale = put_url(
            &service,
            "url-cas-fixture",
            Some("cas-admin-token"),
            payload,
        )
        .await;
        assert_eq!(stale.status_code, Some(StatusCode::CONFLICT));
        let no_op = put_url(
            &service,
            "url-cas-fixture",
            Some("cas-admin-token"),
            json!({"url": target, "expected_url": target}),
        )
        .await;
        assert_eq!(no_op.status_code, Some(StatusCode::OK));

        // Two actual HTTP writers with the same observation must produce exactly
        // one successful rebind, even when they reach separate pool connections.
        let first = "http://relay-a.example/prefix";
        let second = "http://relay-b.example/prefix";
        let (a, b) = tokio::join!(
            put_url(
                &service,
                "url-cas-fixture",
                Some("cas-admin-token"),
                json!({"url": first, "expected_url": target})
            ),
            put_url(
                &service,
                "url-cas-fixture",
                Some("cas-admin-token"),
                json!({"url": second, "expected_url": target})
            ),
        );
        assert!(matches!(
            (a.status_code, b.status_code),
            (Some(StatusCode::OK), Some(StatusCode::CONFLICT))
                | (Some(StatusCode::CONFLICT), Some(StatusCode::OK))
        ));
        expected["url"] = json!(if a.status_code == Some(StatusCode::OK) {
            first
        } else {
            second
        });
        assert_eq!(stored_registration("url-cas-fixture").await, expected);
        let child_after =
            data::user::authenticate_token("cas-child-token", std::time::Duration::ZERO)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(child_after.user.id, virtual_user);
        assert_eq!(
            format!("{:?}", child_after.user),
            format!("{:?}", child_before.user)
        );
        assert_eq!(
            format!("{:?}", child_after.device),
            format!("{:?}", child_before.device)
        );
        assert_eq!(child_after.access_token_id, child_before.access_token_id);

        // SQL NULL is a distinct, explicit original URL, not a wildcard.
        let mut no_url: data::appservice::DbRegistration = serde_json::from_value(before).unwrap();
        no_url.id = "url-cas-null".to_owned();
        no_url.url = None;
        no_url.as_token = "cas-null-as-token".to_owned();
        data::appservice::insert_registration(&no_url)
            .await
            .unwrap();
        let mismatch = put_url(
            &service,
            "url-cas-null",
            Some("cas-admin-token"),
            json!({"url": target, "expected_url": ""}),
        )
        .await;
        assert_eq!(mismatch.status_code, Some(StatusCode::CONFLICT));
        let null_match = put_url(
            &service,
            "url-cas-null",
            Some("cas-admin-token"),
            json!({"url": target, "expected_url": null}),
        )
        .await;
        assert_eq!(null_match.status_code, Some(StatusCode::OK));
    }
}
