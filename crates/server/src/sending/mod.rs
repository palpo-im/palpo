use std::fmt::Debug;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose};
use diesel::prelude::*;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use tokio::sync::{Mutex, Semaphore, mpsc};

use crate::core::appservice::Registration;
use crate::core::appservice::event::{PushEventsReqBody, push_events_request};
use crate::core::events::GlobalAccountDataEventType;
use crate::core::events::push_rules::PushRulesEventContent;
use crate::core::federation::transaction::{Edu, SendMessageReqBody, SendMessageResBody, send_message_request};
use crate::core::identifiers::*;
pub use crate::core::sending::*;
use crate::core::serde::{CanonicalJsonObject, RawJsonValue};
use crate::core::{UnixMillis, push};
use crate::data::connect;
use crate::data::schema::*;
use crate::data::sending::{DbOutgoingRequest, NewDbOutgoingRequest};
use crate::room::{state, timeline};
use crate::sending::resolver::Resolver;
use crate::{AppError, AppResult, ServerConfig, TlsNameMap, config, data, exts::*, utils};

mod dest;
pub use dest::*;
pub mod guard;
pub mod resolver;

const SELECT_PRESENCE_LIMIT: usize = 256;
const SELECT_RECEIPT_LIMIT: usize = 256;
const SELECT_EDU_LIMIT: usize = EDU_LIMIT - 2;
const DEQUEUE_LIMIT: usize = 48;

const EDU_BUF_CAP: usize = 128;
const EDU_VEC_CAP: usize = 1;

pub type EduBuf = Vec<u8>;
pub type EduVec = Vec<EduBuf>;

pub const PDU_LIMIT: usize = 50;
pub const EDU_LIMIT: usize = 100;

// pub(super) type OutgoingItem = (Key, SendingEvent, Destination);
// pub(super) type SendingItem = (Key, SendingEvent);
// pub(super) type QueueItem = (Key, SendingEvent);
// pub(super) type Key = Vec<u8>;
pub static MPSC_SENDER: OnceLock<mpsc::UnboundedSender<(OutgoingKind, SendingEventType, i64)>> = OnceLock::new();
pub static MPSC_RECEIVER: OnceLock<Mutex<mpsc::UnboundedReceiver<(OutgoingKind, SendingEventType, i64)>>> =
    OnceLock::new();

pub fn sender() -> mpsc::UnboundedSender<(OutgoingKind, SendingEventType, i64)> {
    MPSC_SENDER.get().expect("sender should set").clone()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OutgoingKind {
    Appservice(String),
    Push(OwnedUserId, String), // user and pushkey
    Normal(OwnedServerName),
}

impl OutgoingKind {
    pub fn name(&self) -> &'static str {
        match self {
            OutgoingKind::Appservice(_) => "appservice",
            OutgoingKind::Push(_, _) => "push",
            OutgoingKind::Normal(_) => "normal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SendingEventType {
    Pdu(OwnedEventId), // pduid
    Edu(EduBuf),       // pdu json
    Flush,             // none
}

/// The state for a given state hash.
pub fn max_request() -> Arc<Semaphore> {
    static MAX_REQUESTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MAX_REQUESTS
        .get_or_init(|| Arc::new(Semaphore::new(crate::config().max_concurrent_requests as usize)))
        .clone()
}

enum TransactionStatus {
    Running,
    Failed(u32, Instant), // number of times failed, time of last failure
    Retrying(u32),        // number of times failed
}

/// Returns a reqwest client which can be used to send requests
pub fn default_client() -> reqwest::Client {
    // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
    static DEFAULT_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    DEFAULT_CLIENT
        .get_or_init(|| {
            reqwest_client_builder(crate::config())
                .expect("failed to build request clinet")
                .build()
                .expect("failed to build request clinet")
        })
        .clone()
}

/// Returns a client used for resolving .well-knowns
pub fn federation_client() -> reqwest::Client {
    static FEDERATION_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    FEDERATION_CLIENT
        .get_or_init(|| {
            let conf = crate::config();
            // Client is cheap to clone (Arc wrapper) and avoids lifetime issues
            let tls_name_override = Arc::new(RwLock::new(TlsNameMap::new()));

            // let jwt_decoding_key = conf
            //     .jwt_secret
            //     .as_ref()
            //     .map(|secret| jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()));

            reqwest_client_builder(conf)
                .expect("build reqwest client failed")
                .dns_resolver(Arc::new(Resolver::new(tls_name_override.clone())))
                .timeout(Duration::from_secs(2 * 60))
                .build()
                .expect("build reqwest client failed")
        })
        .clone()
}

#[tracing::instrument(skip(pdu_id, user, pushkey))]
pub fn send_push_pdu(pdu_id: &EventId, user: &UserId, pushkey: String) -> AppResult<()> {
    let outgoing_kind = OutgoingKind::Push(user.to_owned(), pushkey);
    let event = SendingEventType::Pdu(pdu_id.to_owned());
    let keys = queue_requests(&[(&outgoing_kind, event.clone())])?;
    sender()
        .send((outgoing_kind, event, keys.into_iter().next().unwrap()))
        .unwrap();

    Ok(())
}

#[tracing::instrument(level = "debug")]
pub fn send_pdu_room(room_id: &RoomId, pdu_id: &EventId) -> AppResult<()> {
    let servers = room_joined_servers::table
        .filter(room_joined_servers::room_id.eq(room_id))
        .filter(room_joined_servers::server_id.ne(config::server_name()))
        .select(room_joined_servers::server_id)
        .load::<OwnedServerName>(&mut connect()?)?;
    send_pdu_servers(servers.into_iter(), pdu_id)
}

#[tracing::instrument(skip(servers, pdu_id), level = "debug")]
pub fn send_pdu_servers<S: Iterator<Item = OwnedServerName>>(servers: S, pdu_id: &EventId) -> AppResult<()> {
    let requests = servers
        .into_iter()
        .map(|server| (OutgoingKind::Normal(server), SendingEventType::Pdu(pdu_id.to_owned())))
        .collect::<Vec<_>>();
    let keys = queue_requests(&requests.iter().map(|(o, e)| (o, e.clone())).collect::<Vec<_>>())?;
    for ((outgoing_kind, event), key) in requests.into_iter().zip(keys) {
        sender().send((outgoing_kind.to_owned(), event, key)).unwrap();
    }

    Ok(())
}

#[tracing::instrument(skip(room_id, edu), level = "debug")]
pub fn send_edu_room(room_id: &RoomId, edu: &Edu) -> AppResult<()> {
    let servers = room_joined_servers::table
        .filter(room_joined_servers::room_id.eq(room_id))
        .filter(room_joined_servers::server_id.ne(config::server_name()))
        .select(room_joined_servers::server_id)
        .load::<OwnedServerName>(&mut connect()?)?;
    send_edu_servers(servers.into_iter(), edu)
}

#[tracing::instrument(skip(servers, edu), level = "debug")]
pub fn send_edu_servers<S: Iterator<Item = OwnedServerName>>(servers: S, edu: &Edu) -> AppResult<()> {
    let mut serialized = EduBuf::new();
    serde_json::to_writer(&mut serialized, &edu).expect("Serialized Edu");

    let requests = servers
        .into_iter()
        .map(|server| {
            (
                OutgoingKind::Normal(server),
                SendingEventType::Edu(serialized.to_owned()),
            )
        })
        .collect::<Vec<_>>();
    let keys = queue_requests(&requests.iter().map(|(o, e)| (o, e.clone())).collect::<Vec<_>>())?;
    for ((outgoing_kind, event), key) in requests.into_iter().zip(keys) {
        sender()
            .send((outgoing_kind.to_owned(), event, key))
            .map_err(|e| AppError::internal(e.to_string()))?;
    }

    Ok(())
}
#[tracing::instrument(skip(server, edu), level = "debug")]
pub fn send_edu_server(server: &ServerName, edu: &Edu) -> AppResult<()> {
    let mut serialized = EduBuf::new();
    serde_json::to_writer(&mut serialized, &edu).expect("Serialized Edu");

    let outgoing_kind = OutgoingKind::Normal(server.to_owned());
    let event = SendingEventType::Edu(serialized.to_owned());
    let key = queue_request(&outgoing_kind, &event)?;
    sender()
        .send((outgoing_kind, event, key))
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(())
}

#[tracing::instrument(skip(server, edu))]
pub fn send_reliable_edu(server: &ServerName, edu: &Edu, id: &str) -> AppResult<()> {
    let mut serialized = EduBuf::new();
    serde_json::to_writer(&mut serialized, &edu).expect("Serialized Edu");

    let outgoing_kind = OutgoingKind::Normal(server.to_owned());
    let event = SendingEventType::Edu(serialized);
    let keys = queue_requests(&[(&outgoing_kind, event.clone())])?;
    sender()
        .send((outgoing_kind, event, keys.into_iter().next().unwrap()))
        .unwrap();

    Ok(())
}

#[tracing::instrument]
pub fn send_pdu_appservice(appservice_id: String, pdu_id: &EventId) -> AppResult<()> {
    let outgoing_kind = OutgoingKind::Appservice(appservice_id);
    let event = SendingEventType::Pdu(pdu_id.to_owned());
    let keys = queue_requests(&[(&outgoing_kind, event.clone())])?;
    sender()
        .send((outgoing_kind, event, keys.into_iter().next().unwrap()))
        .unwrap();

    Ok(())
}

#[tracing::instrument(skip(events, kind))]
async fn send_events(
    kind: OutgoingKind,
    events: Vec<SendingEventType>,
) -> Result<OutgoingKind, (OutgoingKind, AppError)> {
    match &kind {
        OutgoingKind::Appservice(id) => {
            let mut pdu_jsons = Vec::new();
            for event in &events {
                match event {
                    SendingEventType::Pdu(event_id) => pdu_jsons.push(
                        timeline::get_pdu(event_id)
                            .map_err(|e| (kind.clone(), e))?
                            .to_room_event(),
                    ),
                    SendingEventType::Edu(_) => {
                        // Appservices don't need EDUs (?)
                    }
                    SendingEventType::Flush => {}
                }
            }

            let max_request = crate::sending::max_request();
            let permit = max_request.acquire().await;

            let registration = crate::appservice::get_registration(id)
                .map_err(|e| (kind.clone(), e))?
                .ok_or_else(|| {
                    (
                        kind.clone(),
                        AppError::internal("[Appservice] Could not load registration from database"),
                    )
                })?;
            let req_body = PushEventsReqBody { events: pdu_jsons };

            let txn_id =
                &*general_purpose::URL_SAFE_NO_PAD.encode(utils::hash_keys(events.iter().filter_map(|e| match e {
                    SendingEventType::Edu(b) => Some(&**b),
                    SendingEventType::Pdu(b) => Some(b.as_bytes()),
                    SendingEventType::Flush => None,
                })));
            let request = push_events_request(registration.url.as_deref().unwrap_or_default(), txn_id, req_body)
                .map_err(|e| (kind.clone(), e.into()))?
                .into_inner();
            let response = crate::appservice::send_request(registration, request)
                .await
                .map_err(|e| (kind.clone(), e.into()))
                .map(|_response| kind.clone());

            drop(permit);
            response
        }
        OutgoingKind::Push(user_id, pushkey) => {
            let mut pdus = Vec::new();

            for event in &events {
                match event {
                    SendingEventType::Pdu(event_id) => {
                        pdus.push(timeline::get_pdu(event_id).map_err(|e| (kind.clone(), e))?);
                    }
                    SendingEventType::Edu(_) => {
                        // Push gateways don't need EDUs (?)
                    }
                    SendingEventType::Flush => {}
                }
            }

            for pdu in pdus {
                // Redacted events are not notification targets (we don't send push for them)
                if let Some(unsigned) = &pdu.unsigned {
                    if let Ok(unsigned) = serde_json::from_str::<serde_json::Value>(unsigned.get()) {
                        if unsigned.get("redacted_because").is_some() {
                            continue;
                        }
                    }
                }
                let pusher = match data::user::pusher::get_pusher(user_id, pushkey)
                    .map_err(|e| (OutgoingKind::Push(user_id.clone(), pushkey.clone()), e.into()))?
                {
                    Some(pusher) => pusher,
                    None => continue,
                };

                let rules_for_user = data::user::get_global_data::<PushRulesEventContent>(
                    user_id,
                    &GlobalAccountDataEventType::PushRules.to_string(),
                )
                .unwrap_or_default()
                .map(|content: PushRulesEventContent| content.global)
                .unwrap_or_else(|| push::Ruleset::server_default(user_id));

                let unread = crate::room::user::notification_count(user_id, &pdu.room_id)
                    .map_err(|e| (kind.clone(), e.into()))?
                    .try_into()
                    .expect("notification count can't go that high");

                let max_request = crate::sending::max_request();
                let permit = max_request.acquire().await;

                let _response = crate::user::pusher::send_push_notice(user_id, unread, &pusher, rules_for_user, &pdu)
                    .await
                    .map(|_response| kind.clone())
                    .map_err(|e| (kind.clone(), e));

                drop(permit);
            }
            Ok(OutgoingKind::Push(user_id.clone(), pushkey.clone()))
        }
        OutgoingKind::Normal(server) => {
            let mut edu_jsons = Vec::new();
            let mut pdu_jsons = Vec::new();

            for event in &events {
                match event {
                    SendingEventType::Pdu(pdu_id) => {
                        // TODO: check room version and remove event_id if needed
                        let raw = crate::sending::convert_to_outgoing_federation_event(
                            timeline::get_pdu_json(pdu_id)
                                .map_err(|e| (OutgoingKind::Normal(server.clone()), e.into()))?
                                .ok_or_else(|| {
                                    error!("event not found: {server} {pdu_id:?}");
                                    (
                                        OutgoingKind::Normal(server.clone()),
                                        AppError::internal(
                                            "[Normal] Event in servernamevent_datas not found in database.",
                                        ),
                                    )
                                })?,
                        );
                        pdu_jsons.push(raw);
                    }
                    SendingEventType::Edu(edu) => {
                        if let Ok(raw) = serde_json::from_slice(edu) {
                            edu_jsons.push(raw);
                        }
                    }
                    SendingEventType::Flush => {} // flush only; no new content
                }
            }

            let max_request = crate::sending::max_request();
            let permit = max_request.acquire().await;

            let txn_id =
                &*general_purpose::URL_SAFE_NO_PAD.encode(utils::hash_keys(events.iter().filter_map(|e| match e {
                    SendingEventType::Edu(b) => Some(&**b),
                    SendingEventType::Pdu(b) => Some(b.as_bytes()),
                    SendingEventType::Flush => None,
                })));

            let request = send_message_request(
                &server.origin().await,
                txn_id,
                SendMessageReqBody {
                    origin: config::server_name().to_owned(),
                    pdus: pdu_jsons,
                    edus: edu_jsons,
                    origin_server_ts: UnixMillis::now(),
                },
            )
            .map_err(|e| (kind.clone(), e.into()))?
            .into_inner();
            let response = crate::sending::send_federation_request(server, request)
                .await
                .map_err(|e| (kind.clone(), e.into()))?
                .json::<SendMessageResBody>()
                .await
                .map(|response| {
                    for pdu in response.pdus {
                        if pdu.1.is_err() {
                            warn!("Failed to send to {}: {:?}", server, pdu);
                        }
                    }
                    kind.clone()
                })
                .map_err(|e| (kind, e.into()));

            drop(permit);

            response
        }
    }
}

#[tracing::instrument(skip(request))]
pub async fn send_federation_request(
    destination: &ServerName,
    request: reqwest::Request,
) -> AppResult<reqwest::Response> {
    debug!("Waiting for permit");
    let max_request = max_request();
    let permit = max_request.acquire().await;
    debug!("Got permit");
    let url = request.url().clone();
    let response = tokio::time::timeout(
        Duration::from_secs(2 * 60),
        crate::federation::send_request(destination, request),
    )
    .await
    .map_err(|_| {
        warn!("Timeout waiting for server response of {}", url);
        AppError::public("Timeout waiting for server response")
    })?;
    drop(permit);

    response.map_err(Into::into)
}

#[tracing::instrument(skip_all)]
pub async fn send_appservice_request<T>(registration: Registration, request: reqwest::Request) -> AppResult<T>
where
    T: for<'de> Deserialize<'de> + Debug,
{
    // let permit = acquire_request().await;
    let response = crate::appservice::send_request(registration, request).await?;
    // drop(permit);

    Ok(response.json().await?)
}

fn active_requests() -> AppResult<Vec<(i64, OutgoingKind, SendingEventType)>> {
    Ok(outgoing_requests::table
        .filter(outgoing_requests::state.eq("pending"))
        .load::<DbOutgoingRequest>(&mut connect()?)?
        .into_iter()
        .filter_map(|item| {
            let kind = match item.kind.as_str() {
                "appservice" => {
                    if let Some(appservice_id) = &item.appservice_id {
                        OutgoingKind::Appservice(appservice_id.clone())
                    } else {
                        return None;
                    }
                }
                "push" => {
                    if let (Some(user_id), Some(pushkey)) = (item.user_id.clone(), item.pushkey.clone()) {
                        OutgoingKind::Push(user_id, pushkey)
                    } else {
                        return None;
                    }
                }
                "normal" => {
                    if let Some(server_id) = &item.server_id {
                        OutgoingKind::Normal(server_id.to_owned())
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            let event = if let Some(value) = item.edu_json {
                SendingEventType::Edu(value)
            } else if let Some(pdu_id) = item.pdu_id {
                SendingEventType::Pdu(pdu_id)
            } else {
                return None;
            };
            Some((item.id, kind, event))
        })
        .collect())
}

fn delete_request(id: i64) -> AppResult<()> {
    diesel::delete(outgoing_requests::table.find(id)).execute(&mut connect()?)?;
    Ok(())
}

fn delete_all_active_requests_for(outgoing_kind: &OutgoingKind) -> AppResult<()> {
    diesel::delete(
        outgoing_requests::table
            .filter(outgoing_requests::kind.eq(outgoing_kind.name()))
            .filter(outgoing_requests::state.eq("pending")),
    )
    .execute(&mut connect()?)?;

    Ok(())
}

fn delete_all_requests_for(outgoing_kind: &OutgoingKind) -> AppResult<()> {
    diesel::delete(outgoing_requests::table.filter(outgoing_requests::kind.eq(outgoing_kind.name())))
        .execute(&mut connect()?)?;

    Ok(())
}

fn queue_requests(requests: &[(&OutgoingKind, SendingEventType)]) -> AppResult<Vec<i64>> {
    let mut ids = Vec::new();
    for (outgoing_kind, event) in requests {
        ids.push(queue_request(outgoing_kind, event)?);
    }
    Ok(ids)
}
fn queue_request(outgoing_kind: &OutgoingKind, event: &SendingEventType) -> AppResult<i64> {
    let appservice_id = if let OutgoingKind::Appservice(service_id) = outgoing_kind {
        Some(service_id.clone())
    } else {
        None
    };
    let (user_id, pushkey) = if let OutgoingKind::Push(user_id, pushkey) = outgoing_kind {
        (Some(user_id.clone()), Some(pushkey.clone()))
    } else {
        (None, None)
    };
    let server_id = if let OutgoingKind::Normal(server_id) = outgoing_kind {
        Some(server_id.clone())
    } else {
        None
    };
    let (pdu_id, edu_json) = match event {
        SendingEventType::Pdu(pdu_id) => (Some(pdu_id.to_owned()), None),
        SendingEventType::Edu(edu_json) => (None, Some(edu_json.clone())),
        SendingEventType::Flush => (None, None),
    };
    let id = diesel::insert_into(outgoing_requests::table)
        .values(&NewDbOutgoingRequest {
            kind: outgoing_kind.name().to_owned(),
            appservice_id,
            user_id,
            pushkey,
            server_id,
            pdu_id,
            edu_json,
        })
        .returning(outgoing_requests::id)
        .get_result::<i64>(&mut connect()?)?;
    Ok(id)
}

fn active_requests_for(outgoing_kind: &OutgoingKind) -> AppResult<Vec<(i64, SendingEventType)>> {
    let list = outgoing_requests::table
        .filter(outgoing_requests::kind.eq(outgoing_kind.name()))
        .load::<DbOutgoingRequest>(&mut connect()?)?
        .into_iter()
        .filter_map(|r| {
            if let Some(value) = r.edu_json {
                Some((r.id, SendingEventType::Edu(value)))
            } else if let Some(pdu_id) = r.pdu_id {
                Some((r.id, SendingEventType::Pdu(pdu_id)))
            } else {
                None
            }
        })
        .collect();

    Ok(list)
}

fn queued_requests(outgoing_kind: &OutgoingKind) -> AppResult<Vec<(i64, SendingEventType)>> {
    Ok(outgoing_requests::table
        .filter(outgoing_requests::kind.eq(outgoing_kind.name()))
        .load::<DbOutgoingRequest>(&mut connect()?)?
        .into_iter()
        .filter_map(|r| {
            if let Some(value) = r.edu_json {
                Some((r.id, SendingEventType::Edu(value)))
            } else if let Some(pdu_id) = r.pdu_id {
                Some((r.id, SendingEventType::Pdu(pdu_id)))
            } else {
                None
            }
        })
        .collect())
}
fn mark_as_active(events: &[(i64, SendingEventType)]) -> AppResult<()> {
    for (id, e) in events {
        let value = if let SendingEventType::Edu(value) = &e {
            &**value
        } else {
            &[]
        };
        diesel::update(outgoing_requests::table.find(id))
            .set((
                outgoing_requests::data.eq(value),
                outgoing_requests::state.eq("pending"),
            ))
            .execute(&mut connect()?)?;
    }

    Ok(())
}

/// This does not return a full `Pdu` it is only to satisfy palpo's types.
#[tracing::instrument]
pub fn convert_to_outgoing_federation_event(mut pdu_json: CanonicalJsonObject) -> Box<RawJsonValue> {
    if let Some(unsigned) = pdu_json.get_mut("unsigned").and_then(|val| val.as_object_mut()) {
        unsigned.remove("transaction_id");
    }

    pdu_json.remove("event_id");
    pdu_json.remove("event_sn");

    // TODO: another option would be to convert it to a canonical string to validate size
    // and return a Result<RawJson<...>>
    // serde_json::from_str::<RawJson<_>>(
    //     crate::core::serde::to_canonical_json_string(pdu_json).expect("CanonicalJson is valid serde_json::Value"),
    // )
    // .expect("RawJson::from_value always works")

    to_raw_value(&pdu_json).expect("CanonicalJson is valid serde_json::Value")
}

fn reqwest_client_builder(config: &ServerConfig) -> AppResult<reqwest::ClientBuilder> {
    let reqwest_client_builder = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 3));

    // TODO: add proxy support
    // if let Some(proxy) = config.to_proxy()? {
    //     reqwest_client_builder = reqwest_client_builder.proxy(proxy);
    // }

    Ok(reqwest_client_builder)
}
