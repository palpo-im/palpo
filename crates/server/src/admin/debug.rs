use std::{
    collections::HashMap,
    fmt::Write,
    iter::once,
    str::FromStr,
    time::{Instant, SystemTime},
};

use futures_util::{FutureExt, StreamExt, TryStreamExt};
use serde::Serialize;
use tracing_subscriber::EnvFilter;

use crate::core::UnixMillis;
use crate::core::serde::{CanonicalJsonObject, CanonicalJsonValue, RawJsonValue};
use crate::core::{
    EventId, OwnedEventId, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, RoomId, RoomVersionId,
    api::federation::event::get_room_state, events::AnyStateEvent,
};
use crate::{AppError, AppResult, admin::Context, config, info, event::PduEvent, room::timeline, utils};

pub(super) async fn echo(ctx: &Context<'_>, message: Vec<String>) -> AppResult<()> {
    let message = message.join(" ");
    ctx.write_str(&message).await
}

pub(super) async fn get_auth_chain(ctx: &Context<'_>, event_id: OwnedEventId) -> AppResult<()> {
    let Ok(Some(event)) = timeline::get_pdu_json(&event_id).await else {
        return Err(AppError::public("Event not found."));
    };

    let room_id_str = event
        .get("room_id")
        .and_then(CanonicalJsonValue::as_str)
        .ok_or_else(|| Err(AppError::public("Invalid event in database")))?;

    let room_id = <&RoomId>::try_from(room_id_str)
        .map_err(|_| Err(AppError::public("Invalid room id field in event in database")))?;

    let start = Instant::now();
    let count = crate::room::auth_chain::get_auth_chain_ids(room_id, once(event_id.as_ref()))
        .await
        .len();

    let elapsed = start.elapsed();
    let out = format!("Loaded auth chain with length {count} in {elapsed:?}");

    ctx.write_str(&out).await
}

pub(super) async fn parse_pdu(ctx: &Context<'_>) -> AppResult<()> {
    if ctx.body.len() < 2 || !ctx.body[0].trim().starts_with("```") || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let string = ctx.body[1..ctx.body.len().saturating_sub(1)].join("\n");
    match serde_json::from_str(&string) {
        Err(e) => return Err(AppError::public(format!("Invalid json in command body: {e}"))),
        Ok(value) => match crate::core::signatures::reference_hash(&value, &RoomVersionId::V6) {
            Err(e) => return Err(AppError::public(format!("could not parse PDU json: {e:?}"))),
            Ok(hash) => {
                let event_id = EventId::parse(format!("${hash}"));
                match serde_json::from_value::<PduEvent>(serde_json::to_value(value)?) {
                    Err(e) => {
                        return Err(AppError::public(format!(
                            "EventId: {event_id:?}\nCould not parse event: {e}"
                        )));
                    }
                    Ok(pdu) => write!(ctx, "EventId: {event_id:?}\n{pdu:#?}"),
                }
            }
        },
    }
    .await
}

pub(super) async fn get_pdu(ctx: &Context<'_>, event_id: OwnedEventId) -> AppResult<()> {
    let mut outlier = false;
    let mut pdu_json = timeline::get_non_outlier_pdu_json(&event_id).await;

    if pdu_json.is_err() {
        outlier = true;
        pdu_json = timeline::get_pdu_json(&event_id).await;
    }

    match pdu_json {
        Err(_) => return Err(AppError::public("PDU not found locally.")),
        Ok(json) => {
            let text = serde_json::to_string_pretty(&json)?;
            let msg = if outlier {
                "Outlier (Rejected / Soft Failed) PDU found in our database"
            } else {
                "PDU found in our database"
            };
            write!(ctx, "{msg}\n```json\n{text}\n```",)
        }
    }
    .await
}

pub(super) async fn fetch_remote_pdu_list(ctx: &Context<'_>, server: OwnedServerName, force: bool) -> AppResult<()> {
    let conf = config::get();
    if conf.enabled_federation().is_none() {
        return Err(AppError::public("federation is disabled on this homeserver."));
    }

    if server == config::server_name() {
        return Err(AppError::public(
            "Not allowed to send federation requests to ourselves. Please use `get-pdu` for \
			 fetching local PDUs from the database.",
        ));
    }

    if ctx.body.len() < 2 || !ctx.body[0].trim().starts_with("```") || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let list = ctx
        .body
        .iter()
        .collect::<Vec<_>>()
        .drain(1..ctx.body.len().saturating_sub(1))
        .filter_map(|pdu| EventId::parse(pdu).ok())
        .collect::<Vec<_>>();

    let mut failed_count: usize = 0;
    let mut success_count: usize = 0;

    for event_id in list {
        if force {
            match fetch_remote_pdu(ctx, event_id.to_owned(), server.clone()).await {
                Err(e) => {
                    failed_count = failed_count.saturating_add(1);
                    crate::admin::send_text(&format!("Failed to get remote PDU, ignoring error: {e}")).await;

                    warn!("Failed to get remote PDU, ignoring error: {e}");
                }
                _ => {
                    success_count = success_count.saturating_add(1);
                }
            }
        } else {
            fetch_remote_pdu(ctx, event_id.to_owned(), server.clone()).await?;
            success_count = success_count.saturating_add(1);
        }
    }

    let out = format!("Fetched {success_count} remote PDUs successfully with {failed_count} failures");

    ctx.write_str(&out).await
}

pub(super) async fn fetch_remote_pdu(
    ctx: &Context<'_>,
    event_id: OwnedEventId,
    server: OwnedServerName,
) -> AppResult<()> {
    let conf = config::get();
    if conf.enabled_federation().is_none() {
        return Err(AppError::public("Federation is disabled on this homeserver."));
    }

    if server == config::server_name() {
        return Err(AppError::public(format!(
            "Not allowed to send federation requests to ourselves. Please use `get-pdu` for \
			 fetching local PDUs.",
        )));
    }

    unimplemented!()
    // match self
    //     .services
    //     .sending
    //     .send_federation_request(
    //         &server,
    //         crate::core::api::federation::event::get_event::v1::Request {
    //             event_id: event_id.clone(),
    //             include_unredacted_content: None,
    //         },
    //     )
    //     .await
    // {
    //     Err(e) => {
    //         return Err(AppError::public(format!(
    //             "Remote server did not have PDU or failed sending request to remote server: {e}"
    //         )));
    //     }
    //     Ok(response) => {
    //         let json: CanonicalJsonObject = serde_json::from_str(response.pdu.get()).map_err(|e| {
    //             warn!(
    //                 "Requested event ID {event_id} from server but failed to convert from \
	// 					 RawValue to CanonicalJsonObject (malformed event/response?): {e}"
    //             );
    //             AppError::public("Received response from server but failed to parse PDU")
    //         })?;

    //         trace!("Attempting to parse PDU: {:?}", &response.pdu);
    //         let _parsed_pdu = {
    //             let parsed_result = crate::parse_incoming_pdu(&response.pdu)?;

    //             let (event_id, value, room_id) = match parsed_result {
    //                 Ok(t) => t,
    //                 Err(e) => {
    //                     warn!("Failed to parse PDU: {e}");
    //                     info!("Full PDU: {:?}", &response.pdu);
    //                     return Err(AppError::public(format!(
    //                         "Failed to parse PDU remote server {server} sent us: {e}"
    //                     )));
    //                 }
    //             };

    //             vec![(event_id, value, room_id)]
    //         };

    //         info!("Attempting to handle event ID {event_id} as backfilled PDU");
    //         timeline::backfill_pdu(&server, response.pdu).await?;

    //         let text = serde_json::to_string_pretty(&json)?;
    //         let msg = "Got PDU from specified server and handled as backfilled";
    //         write!(ctx, "{msg}. Event body:\n```json\n{text}\n```")
    //     }
    // }
    // .await
}

pub(super) async fn get_room_state(ctx: &Context<'_>, room: OwnedRoomOrAliasId) -> AppResult<()> {
    // TODO: admin
    unimplemented!();
    // let room_id = crate::room::alias::resolve(&room).await?;
    // let room_state: Vec<RawJson<AnyStateEvent>> = crate::room::state::room_state_full_pdus(&room_id)
    //     .map_ok(Event::into_format)
    //     .try_collect()
    //     .await?;

    // if room_state.is_empty() {
    //     return Err(AppError::public(
    //         "Unable to find room state in our database (vector is empty)",
    //     ));
    // }

    // let json = serde_json::to_string_pretty(&room_state).map_err(|e| {
    //     AppError::public(format!(
    //         "Failed to convert room state events to pretty JSON, possible invalid room state \
    // 		 events in our database {e}",
    //     ))
    // })?;

    // let out = format!("```json\n{json}\n```");
    // ctx.write_str(&out).await
}

pub(super) async fn ping(ctx: &Context<'_>, server: OwnedServerName) -> AppResult<()> {
    if server == config::server_name() {
        return Err(AppError::public(
            "Not allowed to send federation requests to ourselves.",
        ));
    }

    let timer = tokio::time::Instant::now();

    unimplemented!()
    // match self
    //     .services
    //     .sending
    //     .send_federation_request(
    //         &server,
    //         crate::core::api::federation::discovery::get_server_version::v1::Request {},
    //     )
    //     .await
    // {
    //     Err(e) => {
    //         return Err(AppError::public(format!(
    //             "Failed sending federation request to specified server:\n\n{e}"
    //         )));
    //     }
    //     Ok(response) => {
    //         let ping_time = timer.elapsed();
    //         let json_text_res = serde_json::to_string_pretty(&response.server);

    //         let out = if let Ok(json) = json_text_res {
    //             format!("Got response which took {ping_time:?} time:\n```json\n{json}\n```")
    //         } else {
    //             format!("Got non-JSON response which took {ping_time:?} time:\n{response:?}")
    //         };

    //         write!(ctx, "{out}")
    //     }
    // }
    // .await
}

pub(super) async fn force_device_list_updates(ctx: &Context<'_>) -> AppResult<()> {
    // Force E2EE device list updates for all users
    for user_id in crate::data::user::all_user_ids().await {
        if let Err(e) = crate::user::mark_device_key_update(user_id).await {
            warn!("Failed to mark device key update for user {user_id}: {e}");
        }
    }

    write!(ctx, "Marked all devices for all users as having new keys to update").await
}

pub(super) async fn change_log_level(ctx: &Context<'_>, filter: Option<String>, reset: bool) -> AppResult<()> {
    let handles = &["console"];
    let conf = config::get();
    if reset {
        let old_filter_layer = match EnvFilter::try_new(&conf.logger.level) {
            Ok(s) => s,
            Err(e) => {
                return Err(AppError::public(format!(
                    "Log level from config appears to be invalid now: {e}"
                )));
            }
        };

        // TODO: This is a workaround for the fact that we cannot reload the logger
        // match crate::config::get().logger.reload(&old_filter_layer, Some(handles)) {
        //     Err(e) => {
        //         return Err(AppError::public(format!(
        //             "Failed to modify and reload the global tracing log level: {e}"
        //         )));
        //     }
        //     Ok(()) => {
        //         let value = &conf.logger.level;
        //         let out = format!("Successfully changed log level back to config value {value}");
        //         return ctx.write_str(&out).await;
        //     }
        // }
    }

    // TODO: This is a workaround for the fact that we cannot reload the logger
    // if let Some(filter) = filter {
    //     let new_filter_layer = match EnvFilter::try_new(filter) {
    //         Ok(s) => s,
    //         Err(e) => return Err(AppError::public(format!("Invalid log level filter specified: {e}"))),
    //     };

    //     match self.services.server.log.reload.reload(&new_filter_layer, Some(handles)) {
    //         Ok(()) => return ctx.write_str("Successfully changed log level").await,
    //         Err(e) => {
    //             return Err(AppError::public(format!(
    //                 "Failed to modify and reload the global tracing log level: {e}"
    //             )));
    //         }
    //     }
    // }

    Err(AppError::public("No log level was specified."))
}

pub(super) async fn sign_json(ctx: &Context<'_>) -> AppResult<()> {
    if ctx.body.len() < 2 || !ctx.body[0].trim().starts_with("```") || ctx.body.last().unwrap_or(&"").trim() != "```" {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let string = ctx.body[1..ctx.body.len().checked_sub(1).unwrap()].join("\n");
    match serde_json::from_str(&string) {
        Err(e) => return Err(AppError::public(format!("invalid json: {e}"))),
        Ok(mut value) => {
            crate::server_key::sign_json(&mut value)?;
            let json_text = serde_json::to_string_pretty(&value)?;
            write!(ctx, "{json_text}")
        }
    }
    .await
}

pub(super) async fn verify_json(ctx: &Context<'_>, room_version: &RoomVersionId) -> AppResult<()> {
    if ctx.body.len() < 2 || !ctx.body[0].trim().starts_with("```") || ctx.body.last().unwrap_or(&"").trim() != "```" {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let string = ctx.body[1..ctx.body.len().checked_sub(1).unwrap()].join("\n");
    match serde_json::from_str::<CanonicalJsonObject>(&string) {
        Err(e) => return Err(AppError::public(format!("invalid json: {e}"))),
        Ok(value) => match crate::server_key::verify_json(&value, room_version).await {
            Err(e) => return Err(AppError::public(format!("signature verification failed: {e}"))),
            Ok(()) => write!(ctx, "Signature correct"),
        },
    }
    .await
}

pub(super) async fn verify_pdu(ctx: &Context<'_>, event_id: OwnedEventId, room_version: &RoomVersionId) -> AppResult<()> {
    use crate::core::signatures::Verified;

    let Some(mut event) = timeline::get_pdu_json(&event_id).await? else {
        return Err(AppError::public("pdu not found in our database."));
    };

    event.remove("event_id");
    let msg = match crate::server_key::verify_event(&event, room_version).await {
        Err(e) => return Err(e),
        Ok(Verified::Signatures) => "signatures OK, but content hash failed (redaction).",
        Ok(Verified::All) => "signatures and hashes OK.",
    };

    ctx.write_str(msg).await
}

pub(super) async fn first_pdu_in_room(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    if !crate::room::is_server_joined(config::server_name(), &room_id).await? {
        return Err(AppError::public(
            "We are not participating in the room / we don't know about the room ID.",
        ));
    }

    unimplemented!()
    // let first_pdu = timeline::first_pdu_in_room(&room_id)
    //     .await
    //     .map_err(|_| AppError::public("Failed to find the first PDU in database"))?;

    // let out = format!("{first_pdu:?}");
    // ctx.write_str(&out).await
}

pub(super) async fn latest_pdu_in_room(ctx: &Context<'_>, room_id: OwnedRoomId) -> AppResult<()> {
    if !crate::room::is_server_joined(config::server_name(), &room_id).await? {
        return Err(AppError::public(
            "We are not participating in the room / we don't know about the room ID.",
        ));
    }

    let latest_pdu = timeline::latest_pdu_in_room(&room_id)
        .await?
        .map_err(|_| AppError::public("Failed to find the latest PDU in database"))?;

    let out = format!("{latest_pdu:?}");
    ctx.write_str(&out).await
}

pub(super) async fn force_set_room_state_from_server(
    ctx: &Context<'_>,
    room_id: OwnedRoomId,
    server_name: OwnedServerName,
) -> AppResult<()> {
    if !crate::room::is_server_joined(config::server_name(), &room_id).await? {
        return Err(AppError::public(
            "We are not participating in the room / we don't know about the room ID.",
        ));
    }

    let first_pdu = timeline::latest_pdu_in_room(&room_id)
        .await
        .map_err(|_| AppError::public("Failed to find the latest PDU in database"))?;

    let room_version = crate::room::get_version(&room_id).await?;

    let mut state: HashMap<u64, OwnedEventId> = HashMap::new();

    let remote_state_response = self
        .services
        .sending
        .send_federation_request(
            &server_name,
            get_room_state::v1::Request {
                room_id: room_id.clone(),
                event_id: first_pdu.event_id().to_owned(),
            },
        )
        .await?;

    for pdu in remote_state_response.pdus.clone() {
        match crate::parse_incoming_pdu(&pdu) {
            Ok(t) => t,
            Err(e) => {
                warn!("could not parse PDU, ignoring: {e}");
                continue;
            }
        };
    }

    info!("Going through room_state response PDUs");
    for result in remote_state_response
        .pdus
        .iter()
        .map(|pdu| crate::server_key::validate_and_add_event_id(pdu, &room_version))
    {
        let Ok((event_id, value)) = result.await else {
            continue;
        };

        let pdu = PduEvent::from_id_val(&event_id, value.clone()).map_err(|e| {
            error!("invalid pdu in fetching remote room state PDUs response: {value:#?}");
            AppError::public(format!("invalid pdu in send_join response: {e:?}"))
        })?;

        // TODO: admin
        // self.services.rooms.outlier.add_pdu_outlier(&event_id, &value);

        // if let Some(state_key) = &pdu.state_key {
        //     let shortstatekey = self
        //         .services
        //         .rooms
        //         .short
        //         .get_or_create_shortstatekey(&pdu.kind.to_string().into(), state_key)
        //         .await;

        //     state.insert(shortstatekey, pdu.event_id.clone());
        // }
    }

    info!("Going through auth_chain response");
    // TODO: admin
    // for result in remote_state_response
    //     .auth_chain
    //     .iter()
    //     .map(|pdu| crate::server_key::validate_and_add_event_id(pdu, &room_version))
    // {
    //     let Ok((event_id, value)) = result.await else {
    //         continue;
    //     };

    //     self.services.rooms.outlier.add_pdu_outlier(&event_id, &value);
    // }

    let new_room_state = crate::event::handler::resolve_state(&room_id, &room_version, state).await?;

    info!("Forcing new room state");
    let HashSetCompressStateEvent {
        shortstatehash: short_state_hash,
        added,
        removed,
    } = crate::room::state::save_state(room_id.clone().as_ref(), new_room_state).await?;

    let state_lock = crate::room::lock_state(&*room_id).await;

    crate::room::state::force_state(room_id.clone().as_ref(), short_state_hash, added, removed)
        .await?;

    info!(
        "Updating joined counts for room just in case (e.g. we may have found a difference in \
		 the room's m.room.member state"
    );
    crate::room::update_currents(&room_id).await?;
    drop(state_lock);
    ctx.write_str("Successfully forced the room state from the requested remote server.")
        .await
}

pub(super) async fn get_signing_keys(
    ctx: &Context<'_>,
    server_name: Option<OwnedServerName>,
    notary: Option<OwnedServerName>,
    query: bool,
) -> AppResult<()> {
    let server_name = server_name.unwrap_or_else(|| config::server_name().to_owned());

    if let Some(notary) = notary {
        let signing_keys = crate::server_key::notary_request(&notary, &server_name).await?;

        let out = format!("```rs\n{signing_keys:#?}\n```");
        return ctx.write_str(&out).await;
    }

    let signing_keys = if query {
        crate::server_key::server_request(&server_name).await?
    } else {
        crate::server_key::signing_keys_for(&server_name).await?
    };

    let out = format!("```rs\n{signing_keys:#?}\n```");
    ctx.write_str(&out).await
}

pub(super) async fn get_verify_keys(ctx: &Context<'_>, server_name: Option<OwnedServerName>) -> AppResult<()> {
    let server_name = server_name.unwrap_or_else(|| config::server_name().to_owned());

    let keys = crate::server_key::verify_keys_for(&server_name).await;

    let mut out = String::new();
    writeln!(out, "| Key ID | Public Key |")?;
    writeln!(out, "| --- | --- |")?;
    for (key_id, key) in keys {
        writeln!(out, "| {key_id} | {key:?} |")?;
    }

    ctx.write_str(&out).await
}

pub(super) async fn resolve_true_destination(
    ctx: &Context<'_>,
    server_name: OwnedServerName,
    no_cache: bool,
) -> AppResult<()> {
    let conf = config::get();
    if conf.enabled_federation().is_none() {
        return Err(AppError::public("Federation is disabled on this homeserver."));
    }

    if server_name == config::server_name() {
        return Err(AppError::public(
            "Not allowed to send federation requests to ourselves. Please use `get-pdu` for \
			 fetching local PDUs.",
        ));
    }

    let actual = self
        .services
        .resolver
        .resolve_actual_dest(&server_name, !no_cache)
        .await?;

    let msg = format!("Destination: {}\nHostname URI: {}", actual.dest, actual.host);
    ctx.write_str(&msg).await
}

pub(super) async fn time(ctx: &Context<'_>) -> AppResult<()> {
    let now = SystemTime::now();
    let now = utils::time::format(now, "%+");

    ctx.write_str(&now).await
}

pub(super) async fn list_dependencies(ctx: &Context<'_>, names: bool) -> AppResult<()> {
    if names {
        let out = info::cargo::dependencies_names().join(" ");
        return ctx.write_str(&out).await;
    }

    let mut out = String::new();
    let deps = info::cargo::dependencies();
    writeln!(out, "| name | version | features |")?;
    writeln!(out, "| ---- | ------- | -------- |")?;
    for (name, dep) in deps {
        let version = dep.try_req().unwrap_or("*");
        let feats = dep.req_features();
        let feats = if !feats.is_empty() {
            feats.join(" ")
        } else {
            String::new()
        };

        writeln!(out, "| {name} | {version} | {feats} |")?;
    }

    ctx.write_str(&out).await
}

pub(super) async fn create_jwt(
    ctx: &Context<'_>,
    user: String,
    exp_from_now: Option<u64>,
    nbf_from_now: Option<u64>,
    issuer: Option<String>,
    audience: Option<String>,
) -> AppResult<()> {
    use jwt::{Algorithm, EncodingKey, Header, encode};

    #[derive(Serialize)]
    struct Claim {
        sub: String,
        iss: String,
        aud: String,
        exp: usize,
        nbf: usize,
    }

    let conf = config::get();

    let Some(jwt_conf) = conf.enabled_jwt() else {
        return Err(AppError::public("JWT is not enabled in the configuration"));
    };
    if jwt_conf.format.as_str() != "HMAC" {
        return Err(AppError::public(format!(
            "This command only supports HMAC key format, not {}.",
            jwt_conf.format
        )));
    }

    let key = EncodingKey::from_secret(jwt_conf.secret.as_ref());
    let alg = Algorithm::from_str(jwt_conf.algorithm.as_str())
        .map_err(|e| AppError::public(format!("JWT algorithm is not recognized or configured {e}")))?;

    let header = Header {
        alg,
        ..Default::default()
    };
    let claim = Claim {
        sub: user,
        iss: issuer.unwrap_or_default(),
        aud: audience.unwrap_or_default(),
        exp: exp_from_now
            .and_then(|val| UnixMillis::now().as_secs().checked_add(val))
            .map(TryInto::try_into)
            .and_then(Result::ok)
            .unwrap_or(usize::MAX),
        nbf: nbf_from_now
            .and_then(|val| UnixMillis::now().as_secs().checked_add(val))
            .map(TryInto::try_into)
            .and_then(Result::ok)
            .unwrap_or(0),
    };

    encode(&header, &claim, &key)
        .map_err(|e| AppError::public(format!("Failed to encode JWT: {e}")))
        .map(async |token| ctx.write_str(&token).await)?
        .await
}
