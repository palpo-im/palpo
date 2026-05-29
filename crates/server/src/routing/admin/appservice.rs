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

pub fn router() -> Router {
    Router::with_path("v1").push(
        Router::with_path("appservices")
            .get(list_appservices)
            .post(register_appservice)
            .push(
                Router::with_path("{id}")
                    .get(get_appservice)
                    .delete(delete_appservice)
                    .push(Router::with_path("disable").post(disable_appservice))
                    .push(Router::with_path("enable").post(enable_appservice)),
            ),
    )
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
