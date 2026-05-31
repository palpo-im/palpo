//! Admin Debug API
//!
//! These endpoints expose the usable parts of the old admin-room debug
//! commands as admin-only JSON APIs for management UIs.

use std::time::{Instant, SystemTime};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use reqwest::Url;
use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::core::serde::canonical_json::{CanonicalJsonObject, CanonicalJsonValue};
use crate::core::{RoomVersionId, UnixMillis};
use crate::data::schema::*;
use crate::event::PduEvent;
use crate::exts::GetUrlOrigin;
use crate::room::{state, timeline};
use crate::{AppError, AppResult, JsonResult, MatrixError, config, info, json_ok, sending, utils};

pub fn router() -> Router {
    Router::with_path("v1/debug")
        .push(Router::with_path("time").get(time))
        .push(Router::with_path("dependencies").get(list_dependencies))
        .push(Router::with_path("device_list_updates").post(force_device_list_updates))
        .push(
            Router::with_path("json")
                .push(Router::with_path("sign").post(sign_json))
                .push(Router::with_path("verify").post(verify_json)),
        )
        .push(
            Router::with_path("pdu")
                .push(Router::with_path("parse").post(parse_pdu))
                .push(
                    Router::with_path("{event_id}")
                        .get(get_pdu)
                        .push(Router::with_path("auth_chain").get(get_auth_chain))
                        .push(Router::with_path("verify").post(verify_pdu)),
                ),
        )
        .push(
            Router::with_path("rooms/{room_id}")
                .push(Router::with_path("state").get(get_room_state))
                .push(Router::with_path("first_pdu").get(first_pdu_in_room))
                .push(Router::with_path("latest_pdu").get(latest_pdu_in_room)),
        )
        .push(
            Router::with_path("federation/{server_name}")
                .push(Router::with_path("ping").get(ping))
                .push(
                    Router::with_path("keys")
                        .push(Router::with_path("signing").get(get_signing_keys))
                        .push(Router::with_path("verify").get(get_verify_keys)),
                ),
        )
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeResponse {
    pub time: String,
    pub unix_millis: u64,
}

#[endpoint]
pub fn time() -> JsonResult<TimeResponse> {
    json_ok(TimeResponse {
        time: utils::time::format(SystemTime::now(), "%+"),
        unix_millis: UnixMillis::now().get().into(),
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DependencyInfo {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DependenciesResponse {
    pub names: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<DependencyInfo>>,
}

#[endpoint]
pub fn list_dependencies(names_only: QueryParam<bool, false>) -> JsonResult<DependenciesResponse> {
    let names_only = names_only.into_inner().unwrap_or(false);
    let names = info::cargo::dependencies_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();

    if names_only {
        return json_ok(DependenciesResponse {
            names,
            dependencies: None,
        });
    }

    let dependencies = info::cargo::dependencies()
        .iter()
        .map(|(name, dep)| DependencyInfo {
            name: name.to_owned(),
            version: dep.try_req().unwrap_or("*").to_owned(),
            features: dep
                .req_features()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
        })
        .collect();

    json_ok(DependenciesResponse {
        names,
        dependencies: Some(dependencies),
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PduResponse {
    pub event_id: String,
    pub outlier: bool,
    pub event: serde_json::Value,
}

#[endpoint]
pub async fn get_pdu(event_id: PathParam<OwnedEventId>) -> JsonResult<PduResponse> {
    let event_id = event_id.into_inner();

    if let Ok(Some(pdu)) = timeline::get_non_outlier_pdu(&event_id).await {
        return json_ok(PduResponse {
            event_id: event_id.to_string(),
            outlier: false,
            event: serde_json::to_value(pdu)?,
        });
    }

    let event = timeline::get_pdu_json(&event_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("PDU not found locally"))?;

    json_ok(PduResponse {
        event_id: event_id.to_string(),
        outlier: true,
        event: serde_json::to_value(event)?,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthChainResponse {
    pub event_id: String,
    pub room_id: String,
    pub count: usize,
    pub elapsed_ms: u128,
}

#[endpoint]
pub async fn get_auth_chain(event_id: PathParam<OwnedEventId>) -> JsonResult<AuthChainResponse> {
    let event_id = event_id.into_inner();
    let Some(event) = timeline::get_pdu_json(&event_id).await? else {
        return Err(MatrixError::not_found("Event not found").into());
    };

    let room_id_str = event
        .get("room_id")
        .and_then(CanonicalJsonValue::as_str)
        .ok_or_else(|| AppError::public("Invalid event in database: missing room_id"))?;
    let room_id = <&RoomId>::try_from(room_id_str)
        .map_err(|_| AppError::public("Invalid event in database: invalid room_id"))?;

    let start = Instant::now();
    let count =
        crate::room::auth_chain::get_auth_chain_ids(room_id, std::iter::once(event_id.as_ref()))
            .await?
            .len();

    json_ok(AuthChainResponse {
        event_id: event_id.to_string(),
        room_id: room_id.to_string(),
        count,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ParsePduReqBody {
    pub event: serde_json::Value,
    #[serde(default)]
    pub room_version: Option<RoomVersionId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ParsePduResponse {
    pub event_id: String,
    pub room_version: String,
    pub parsed: serde_json::Value,
}

#[endpoint]
pub async fn parse_pdu(body: JsonBody<ParsePduReqBody>) -> JsonResult<ParsePduResponse> {
    let body = body.into_inner();
    let room_version = body.room_version.unwrap_or(RoomVersionId::V6);
    let event = serde_json::from_value::<CanonicalJsonObject>(body.event)
        .map_err(|e| AppError::public(format!("invalid canonical json object: {e}")))?;
    let rules = crate::room::get_version_rules(&room_version)?;
    let hash = crate::core::signatures::reference_hash(&event, &rules)
        .map_err(|e| AppError::public(format!("could not hash PDU json: {e:?}")))?;
    let event_id = EventId::parse(format!("${hash}"))
        .map_err(|e| AppError::public(format!("generated event ID is invalid: {e}")))?;
    let pdu = serde_json::from_value::<PduEvent>(serde_json::to_value(event)?)
        .map_err(|e| AppError::public(format!("could not parse event: {e}")))?;

    json_ok(ParsePduResponse {
        event_id: event_id.to_string(),
        room_version: room_version.as_str().to_owned(),
        parsed: serde_json::to_value(pdu)?,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoomStateResponse {
    pub room_id: String,
    pub events: Vec<serde_json::Value>,
    pub total: usize,
}

#[endpoint]
pub async fn get_room_state(room_id: PathParam<OwnedRoomId>) -> JsonResult<RoomStateResponse> {
    let room_id = room_id.into_inner();
    let frame_id = crate::room::get_frame_id(&room_id, None)
        .await
        .unwrap_or_default();
    let events = state::get_full_state(frame_id)
        .await?
        .values()
        .map(|pdu| serde_json::to_value(pdu.to_state_event()).unwrap_or_default())
        .collect::<Vec<_>>();

    json_ok(RoomStateResponse {
        room_id: room_id.to_string(),
        total: events.len(),
        events,
    })
}

#[endpoint]
pub async fn first_pdu_in_room(room_id: PathParam<OwnedRoomId>) -> JsonResult<PduResponse> {
    let room_id = room_id.into_inner();
    let pdu = timeline::first_pdu_in_room(&room_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("No PDU found in room"))?;
    json_ok(pdu_response(pdu, false)?)
}

#[endpoint]
pub async fn latest_pdu_in_room(room_id: PathParam<OwnedRoomId>) -> JsonResult<PduResponse> {
    let room_id = room_id.into_inner();
    let pdu = latest_pdu(&room_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("No PDU found in room"))?;
    json_ok(pdu_response(pdu, false)?)
}

async fn latest_pdu(room_id: &RoomId) -> AppResult<Option<PduEvent>> {
    event_datas::table
        .filter(event_datas::room_id.eq(room_id))
        .order(event_datas::event_sn.desc())
        .select((event_datas::event_id, event_datas::json_data))
        .first::<(OwnedEventId, JsonValue)>(&mut crate::data::connect().await?)
        .await
        .optional()?
        .map(|(event_id, json)| {
            PduEvent::from_json_value(room_id, &event_id, json)
                .map_err(|_| AppError::internal("invalid pdu in db"))
        })
        .transpose()
}

fn pdu_response(pdu: PduEvent, outlier: bool) -> AppResult<PduResponse> {
    Ok(PduResponse {
        event_id: pdu.event_id.to_string(),
        outlier,
        event: serde_json::to_value(pdu)?,
    })
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct JsonDebugReqBody {
    pub json: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyJsonReqBody {
    pub json: serde_json::Value,
    #[serde(default)]
    pub room_version: Option<RoomVersionId>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyPduReqBody {
    #[serde(default)]
    pub room_version: Option<RoomVersionId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JsonDebugResponse {
    pub json: serde_json::Value,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyResponse {
    pub valid: bool,
    pub room_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[endpoint]
pub async fn sign_json(body: JsonBody<JsonDebugReqBody>) -> JsonResult<JsonDebugResponse> {
    let mut value = serde_json::from_value::<CanonicalJsonObject>(body.into_inner().json)
        .map_err(|e| AppError::public(format!("invalid canonical json object: {e}")))?;
    crate::server_key::sign_json(&mut value)?;

    json_ok(JsonDebugResponse {
        json: serde_json::to_value(value)?,
    })
}

#[endpoint]
pub async fn verify_json(body: JsonBody<VerifyJsonReqBody>) -> JsonResult<VerifyResponse> {
    let body = body.into_inner();
    let room_version = body.room_version.unwrap_or(RoomVersionId::V6);
    let value = serde_json::from_value::<CanonicalJsonObject>(body.json)
        .map_err(|e| AppError::public(format!("invalid canonical json object: {e}")))?;

    crate::server_key::verify_json(&value, &room_version).await?;

    json_ok(VerifyResponse {
        valid: true,
        room_version: room_version.as_str().to_owned(),
        detail: None,
    })
}

#[endpoint]
pub async fn verify_pdu(
    event_id: PathParam<OwnedEventId>,
    body: JsonBody<VerifyPduReqBody>,
) -> JsonResult<VerifyResponse> {
    use crate::core::signatures::Verified;

    let event_id = event_id.into_inner();
    let room_version = body.into_inner().room_version.unwrap_or(RoomVersionId::V6);
    let Some(mut event) = timeline::get_pdu_json(&event_id).await? else {
        return Err(MatrixError::not_found("PDU not found locally").into());
    };

    event.remove("event_id");
    let detail = match crate::server_key::verify_event(&event, &room_version).await? {
        Verified::Signatures => "signatures OK, but content hash failed (redaction).",
        Verified::All => "signatures and hashes OK.",
    };

    json_ok(VerifyResponse {
        valid: true,
        room_version: room_version.as_str().to_owned(),
        detail: Some(detail.to_owned()),
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PingResponse {
    pub server_name: String,
    pub elapsed_ms: u128,
    pub status: u16,
    pub response: serde_json::Value,
}

#[endpoint]
pub async fn ping(server_name: PathParam<OwnedServerName>) -> JsonResult<PingResponse> {
    let server_name = server_name.into_inner();
    if config::get().enabled_federation().is_none() {
        return Err(AppError::public("Federation is disabled on this homeserver").into());
    }
    if server_name == config::server_name() {
        return Err(
            AppError::public("Not allowed to send federation requests to ourselves").into(),
        );
    }
    reject_ip_literal(&server_name)?;

    let url = Url::parse(&format!(
        "{}/_matrix/federation/v1/version",
        server_name.origin().await
    ))?;
    let request = crate::core::sending::get(url).into_inner();
    let start = Instant::now();
    let response = sending::send_federation_request(&server_name, request, Some(30)).await?;
    let status = response.status().as_u16();
    let body = response.json::<serde_json::Value>().await?;

    json_ok(PingResponse {
        server_name: server_name.to_string(),
        elapsed_ms: start.elapsed().as_millis(),
        status,
        response: serde_json::to_value(body)?,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SigningKeysResponse {
    pub server_name: String,
    pub source: String,
    pub keys: serde_json::Value,
}

#[endpoint]
pub async fn get_signing_keys(
    server_name: PathParam<OwnedServerName>,
    notary: QueryParam<OwnedServerName, false>,
    query: QueryParam<bool, false>,
) -> JsonResult<SigningKeysResponse> {
    let server_name = server_name.into_inner();

    if let Some(notary) = notary.into_inner() {
        reject_ip_literal(&notary)?;
        let keys = crate::server_key::notary_request(&notary, &server_name)
            .await?
            .collect::<Vec<_>>();
        return json_ok(SigningKeysResponse {
            server_name: server_name.to_string(),
            source: format!("notary:{notary}"),
            keys: serde_json::to_value(keys)?,
        });
    }

    let (source, keys) = if query.into_inner().unwrap_or(false) {
        reject_ip_literal(&server_name)?;
        (
            "origin".to_owned(),
            serde_json::to_value(crate::server_key::server_request(&server_name).await?)?,
        )
    } else {
        (
            "cache".to_owned(),
            serde_json::to_value(crate::server_key::signing_keys_for(&server_name).await?)?,
        )
    };

    json_ok(SigningKeysResponse {
        server_name: server_name.to_string(),
        source,
        keys,
    })
}

fn reject_ip_literal(server_name: &ServerName) -> AppResult<()> {
    if server_name.is_ip_literal() {
        return Err(MatrixError::forbidden(
            "Federation debug requests to IP-literal server names are not allowed",
            None,
        )
        .into());
    }

    Ok(())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyKeyInfo {
    pub key_id: String,
    pub key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyKeysResponse {
    pub server_name: String,
    pub keys: Vec<VerifyKeyInfo>,
}

#[endpoint]
pub async fn get_verify_keys(
    server_name: PathParam<OwnedServerName>,
) -> JsonResult<VerifyKeysResponse> {
    let server_name = server_name.into_inner();
    let keys = crate::server_key::verify_keys_for(&server_name)
        .await
        .into_iter()
        .map(|(key_id, key)| VerifyKeyInfo {
            key_id: key_id.to_string(),
            key: format!("{:?}", key.key),
        })
        .collect();

    json_ok(VerifyKeysResponse {
        server_name: server_name.to_string(),
        keys,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeviceListUpdateFailure {
    pub user_id: String,
    pub error: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ForceDeviceListUpdatesResponse {
    pub users_seen: usize,
    pub users_marked: usize,
    pub devices_seen: usize,
    pub failures: Vec<DeviceListUpdateFailure>,
}

#[endpoint]
pub async fn force_device_list_updates() -> JsonResult<ForceDeviceListUpdatesResponse> {
    let users = crate::data::user::list_local_users().await?;
    let mut users_marked = 0usize;
    let mut devices_seen = 0usize;
    let mut failures = Vec::new();

    for user_id in &users {
        let result = async {
            let devices = crate::data::user::all_device_ids(user_id).await?;
            devices_seen = devices_seen.saturating_add(devices.len());

            if let Some(device_id) = devices.get(0) {
                crate::user::mark_device_key_update(user_id, device_id).await
            } else {
                crate::user::mark_signing_key_update(user_id).await
            }
        }
        .await;

        match result {
            Ok(()) => users_marked = users_marked.saturating_add(1),
            Err(e) => failures.push(DeviceListUpdateFailure {
                user_id: user_id.to_string(),
                error: e.to_string(),
            }),
        }
    }

    json_ok(ForceDeviceListUpdatesResponse {
        users_seen: users.len(),
        users_marked,
        devices_seen,
        failures,
    })
}
