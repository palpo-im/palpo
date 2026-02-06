use std::collections::{BTreeMap, HashSet};

use diesel::prelude::*;
use serde_json::value::to_raw_value;

use crate::core::Direction;
use crate::core::client::message::{
    CreateMessageReqArgs, CreateMessageWithTxnReqArgs, MessagesReqArgs, MessagesResBody,
    SendMessageResBody,
};
use crate::core::events::{StateEventType, TimelineEventType};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::BatchToken;
use crate::room::timeline::{self, topolo};
use crate::routing::prelude::*;
use crate::{PduBuilder, room};

/// #GET /_matrix/client/r0/rooms/{room_id}/messages
/// Allows paginating through room history.
#[endpoint]
pub(super) async fn get_messages(
    _aa: AuthArgs,
    args: MessagesReqArgs,
    depot: &mut Depot,
) -> JsonResult<MessagesResBody> {
    let authed = depot.authed_info()?;
    let sender_id = authed.user_id();

    let is_joined = diesel_exists!(
        room_users::table
            .filter(room_users::room_id.eq(&args.room_id))
            .filter(room_users::user_id.eq(sender_id))
            .filter(room_users::membership.eq("join")),
        &mut connect()?
    )?;
    if !is_joined {
        let forgotten_row = room_users::table
            .filter(room_users::room_id.eq(&args.room_id))
            .filter(room_users::user_id.eq(sender_id))
            .filter(room_users::membership.eq("leave"))
            .select((room_users::event_sn, room_users::forgotten))
            .first::<(i64, bool)>(&mut connect()?);
        let Some((_event_sn, forgotten)) = diesel::OptionalExtension::optional(forgotten_row)? else {
            return Err(MatrixError::forbidden("you aren't a member of the room", None).into());
        };
        if forgotten {
            return Err(MatrixError::forbidden("you aren't a member of the room", None).into());
        }
    }
    // let until_tk = if !is_joined {
    //     let Some((event_sn, forgotten)) = room_users::table
    //         .filter(room_users::room_id.eq(&args.room_id))
    //         .filter(room_users::user_id.eq(sender_id))
    //         .filter(room_users::membership.eq("leave"))
    //         .select((room_users::event_sn, room_users::forgotten))
    //         .first::<(i64, bool)>(&mut connect()?)
    //         .optional()?
    //     else {
    //         return Err(MatrixError::forbidden("you aren't a member of the room", None).into());
    //     };
    //     if forgotten {
    //         return Err(MatrixError::forbidden("you aren't a member of the room", None).into());
    //     }
    //     get_batch_token_by_sn(event_sn).ok()
    // } else {
    //     args.to.as_ref().map(|to| to.parse()).transpose()?
    // };
    let until_tk = args.to.as_ref().map(|to| to.parse()).transpose()?;

    let from_tk: BatchToken = args
        .from
        .as_ref()
        .map(|from| from.parse())
        .transpose()?
        .unwrap_or(match args.dir {
            Direction::Forward => BatchToken::LIVE_MIN,
            Direction::Backward => BatchToken::LIVE_MAX,
        });
    // if from_tk.event_depth.is_none() {
    //     from_tk = events::table
    //         .filter(events::sn.le(from_tk.event_sn()))
    //         .order_by(events::sn.desc())
    //         .select((events::sn, events::depth))
    //         .first::<(Seqnum, i64)>(&mut connect()?)
    //         .map(|(sn, depth)| BatchToken::new(sn, Some(depth)))?
    // }

    crate::room::lazy_loading::lazy_load_confirm_delivery(
        authed.user_id(),
        authed.device_id(),
        &args.room_id,
        from_tk.event_sn(),
    )?;

    let limit = args.limit.min(100);
    let next_token;
    let mut resp = MessagesResBody::default();
    let mut lazy_loaded = HashSet::new();
    match args.dir {
        Direction::Forward => {
            let events = topolo::load_pdus_forward(
                Some(sender_id),
                &args.room_id,
                Some(from_tk),
                until_tk,
                Some(&args.filter),
                limit,
            )?;

            for (_, event) in &events {
                // TODO: Remove this when these are resolved:
                // https://github.com/vector-im/element-android/issues/3417
                // https://github.com/vector-im/element-web/issues/21034
                // if !crate::room::lazy_loading.lazy_load_was_sent_before(
                // sender_id,
                // sender_id,
                // &body.room_id,
                // &event.sender,
                // )? {
                // lazy_loaded.insert(event.sender.clone());
                // }
                lazy_loaded.insert(event.sender.clone());
            }

            next_token = events.last().map(|(_, pdu)| pdu.live_token());

            let events: Vec<_> = events
                .into_iter()
                .map(|(_, pdu)| pdu.to_room_event())
                .collect();

            resp.start = from_tk.to_string();
            resp.end = next_token.map(|tk| tk.to_string());
            resp.chunk = events;
        }
        Direction::Backward => {
            let mut events: indexmap::IndexMap<i64, crate::SnPduEvent> =
                topolo::load_pdus_backward(
                    Some(sender_id),
                    &args.room_id,
                    Some(from_tk),
                    until_tk,
                    Some(&args.filter),
                    limit,
                )?;
            let filled_events =
                timeline::backfill_if_required(&args.room_id, &from_tk, &events, limit).await?;
            if !filled_events.is_empty() {
                events = topolo::load_pdus_backward(
                    Some(sender_id),
                    &args.room_id,
                    Some(from_tk),
                    until_tk,
                    Some(&args.filter),
                    limit,
                )?;
            }

            for (_, event) in &events {
                // TODO: Remove this when these are resolved:
                // https://github.com/vector-im/element-android/issues/3417
                // https://github.com/vector-im/element-web/issues/21034
                // if !crate::room::lazy_loading.lazy_load_was_sent_before(
                // sender_id,
                // authed.device_id(),
                // &args.room_id,
                // &event.sender,
                // )? {
                // lazy_loaded.insert(event.sender.clone());
                // }
                lazy_loaded.insert(event.sender.clone());
            }
            next_token = events.last().map(|(_, pdu)| pdu.prev_historic_token());
            resp.start = from_tk.to_string();
            resp.end = next_token.map(|tk| tk.to_string());
            resp.chunk = events.values().map(|pdu| pdu.to_room_event()).collect();
        }
    }

    resp.state = Vec::new();
    for ll_id in &lazy_loaded {
        if let Ok(member_event) = room::get_state(
            &args.room_id,
            &StateEventType::RoomMember,
            ll_id.as_str(),
            None,
        ) {
            resp.state.push(member_event.to_state_event());
        }
    }

    // TODO: enable again when we are sure clients can handle it
    // if let Some(next_token) = next_token {
    // crate::room::lazy_loading.lazy_load_mark_sent(
    // authed.user_id(),
    // authed.device_id(),
    // &body.room_id,
    // lazy_loaded,
    // next_token,
    // );
    // }

    json_ok(resp)
}

/// #PUT /_matrix/client/r0/rooms/{room_id}/send/{event_type}/{txn_id}
/// Send a message event into the room.
///
/// - Is a NOOP if the txn id was already used before and returns the same event id again
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is allowed
#[endpoint]
pub(super) async fn send_message(
    _aa: AuthArgs,
    args: CreateMessageWithTxnReqArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<SendMessageResBody> {
    let authed = depot.authed_info()?;

    let conf = config::get();
    // Forbid m.room.encrypted if encryption is disabled
    if TimelineEventType::RoomEncrypted == args.event_type.to_string().into()
        && !conf.allow_encryption
    {
        return Err(MatrixError::forbidden("Encryption has been disabled", None).into());
    }

    let payload = req.payload().await?;
    // Ensure it's valid JSON.
    let _content: JsonValue =
        serde_json::from_slice(payload).map_err(|_| MatrixError::bad_json("invalid json body"))?;

    let state_lock = room::lock_state(&args.room_id).await;
    // Check if this is a new transaction id
    if let Some(event_id) = crate::transaction_id::get_event_id(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
        Some(&args.room_id),
    )? {
        return json_ok(SendMessageResBody::new(event_id));
    }

    let mut unsigned = BTreeMap::new();
    unsigned.insert(
        "transaction_id".to_owned(),
        to_raw_value(&args.txn_id).expect("TxnId is valid json"),
    );

    let event_id = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: args.event_type.to_string().into(),
            content: serde_json::from_slice(payload)
                .map_err(|_| MatrixError::bad_json("invalid json body"))?,
            unsigned,
            timestamp: if authed.appservice().is_some() {
                args.timestamp
            } else {
                None
            },
            ..Default::default()
        },
        authed.user_id(),
        &args.room_id,
        &crate::room::get_version(&args.room_id)?,
        &state_lock,
    )
    .await?
    .pdu
    .event_id;

    crate::transaction_id::add_txn_id(
        &args.txn_id,
        authed.user_id(),
        Some(authed.device_id()),
        Some(&args.room_id),
        Some(&event_id),
    )?;

    json_ok(SendMessageResBody::new((*event_id).to_owned()))
}

/// #POST /_matrix/client/r0/rooms/{room_id}/send/{event_type}
/// Send a message event into the room.
///
/// - Is a NOOP if the txn id was already used before and returns the same event id again
/// - The only requirement for the content is that it has to be valid json
/// - Tries to send the event into the room, auth rules will determine if it is allowed
#[endpoint]
pub(super) async fn post_message(
    _aa: AuthArgs,
    args: CreateMessageReqArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<SendMessageResBody> {
    let authed = depot.authed_info()?;

    let conf = config::get();
    let state_lock = room::lock_state(&args.room_id).await;
    // Forbid m.room.encrypted if encryption is disabled
    if TimelineEventType::RoomEncrypted == args.event_type.to_string().into()
        && !conf.allow_encryption
    {
        return Err(MatrixError::forbidden("Encryption has been disabled", None).into());
    }

    let payload = req.payload().await?;
    // Ensure it's valid JSON.
    let content: JsonValue =
        serde_json::from_slice(payload).map_err(|_| MatrixError::bad_json("invalid json body"))?;
    if !content.is_object() {
        return Err(MatrixError::bad_json("json body is not object").into());
    }

    let event_id = timeline::build_and_append_pdu(
        PduBuilder {
            event_type: args.event_type.to_string().into(),
            content: serde_json::from_slice(payload)
                .map_err(|_| MatrixError::bad_json("invalid json body"))?,
            unsigned: BTreeMap::new(),
            ..Default::default()
        },
        authed.user_id(),
        &args.room_id,
        &crate::room::get_version(&args.room_id)?,
        &state_lock,
    )
    .await?
    .pdu
    .event_id;

    json_ok(SendMessageResBody::new((*event_id).to_owned()))
}
