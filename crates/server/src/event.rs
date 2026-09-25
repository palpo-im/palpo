mod batch_token;
pub mod fetching;
pub mod handler;
mod pdu;
pub mod resolver;
pub use batch_token::*;
pub use pdu::*;
mod outlier;
pub mod search;
pub mod sticky;
use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
pub use outlier::*;

use crate::core::identifiers::*;
use crate::core::room_version_rules::RoomIdFormatVersion;
use crate::core::serde::{CanonicalJsonObject, RawJsonValue};
use crate::core::{Direction, Seqnum, UnixMillis, signatures};
use crate::data::connect;
use crate::data::room::DbEvent;
use crate::data::schema::*;
use crate::utils::SeqnumQueueGuard;
use crate::{AppError, AppResult, MatrixError};

/// Generates a correct eventId for the incoming pdu.
///
/// Returns a tuple of the new `EventId` and the PDU as a `BTreeMap<String, CanonicalJsonValue>`.
pub fn gen_event_id_canonical_json(
    pdu: &RawJsonValue,
    room_version_id: &RoomVersionId,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
        warn!("error parsing event {:?}: {:?}", pdu, e);
        AppError::public("invalid pdu in server response")
    })?;
    let event_id = gen_event_id(&value, room_version_id)?;
    Ok((event_id, value))
}
/// Generates a correct eventId for the incoming pdu.
pub fn gen_event_id(
    value: &CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> AppResult<OwnedEventId> {
    let version_rules = crate::room::get_version_rules(room_version_id)?;
    let reference_hash = signatures::reference_hash(value, &version_rules)?;
    let event_id: OwnedEventId = format!("${reference_hash}").try_into()?;
    Ok(event_id)
}

pub async fn ensure_event_sn(
    room_id: &RoomId,
    event_id: &EventId,
) -> AppResult<(Seqnum, Option<SeqnumQueueGuard>)> {
    if let Some(sn) = event_points::table
        .find(event_id)
        .select(event_points::event_sn)
        .first::<Seqnum>(&mut connect().await?)
        .await
        .optional()?
    {
        Ok((sn, None))
    } else {
        let sn = diesel::insert_into(event_points::table)
            .values((
                event_points::event_id.eq(event_id),
                event_points::room_id.eq(room_id),
            ))
            .on_conflict_do_nothing()
            .returning(event_points::event_sn)
            .get_result::<Seqnum>(&mut connect().await?)
            .await?;

        diesel::update(events::table.find(event_id))
            .set(events::sn.eq(sn))
            .execute(&mut connect().await?)
            .await?;

        diesel::update(event_datas::table.find(event_id))
            .set(event_datas::event_sn.eq(sn))
            .execute(&mut connect().await?)
            .await?;

        Ok((sn, Some(crate::queue_seqnum(sn))))
    }
}
/// Returns the `count` of this pdu's id.
pub async fn get_event_sn(event_id: &EventId) -> AppResult<Seqnum> {
    event_points::table
        .find(event_id)
        .select(event_points::event_sn)
        .first::<Seqnum>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn get_live_token(event_id: &EventId) -> AppResult<BatchToken> {
    events::table
        .find(event_id)
        .select((events::sn, events::depth))
        .first::<(Seqnum, i64)>(&mut connect().await?)
        .await
        .map(|(sn, _depth)| BatchToken::new_live(sn))
        .map_err(Into::into)
}
pub async fn get_historic_token(event_id: &EventId) -> AppResult<BatchToken> {
    events::table
        .find(event_id)
        .select((events::sn, events::depth))
        .first::<(Seqnum, i64)>(&mut connect().await?)
        .await
        .map(|(sn, depth)| BatchToken::new_historic(sn, depth))
        .map_err(Into::into)
}
pub async fn get_historic_token_by_sn(event_sn: Seqnum) -> AppResult<BatchToken> {
    events::table
        .filter(events::sn.eq(event_sn))
        .select((events::sn, events::depth))
        .first::<(Seqnum, i64)>(&mut connect().await?)
        .await
        .map(|(sn, depth)| BatchToken::new_historic(sn, depth))
        .map_err(Into::into)
}

pub async fn get_event_id_by_sn(event_sn: Seqnum) -> AppResult<OwnedEventId> {
    event_points::table
        .filter(event_points::event_sn.eq(event_sn))
        .select(event_points::event_id)
        .first::<OwnedEventId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn get_event_for_timestamp(
    room_id: &RoomId,
    timestamp: UnixMillis,
    dir: Direction,
) -> AppResult<(OwnedEventId, UnixMillis)> {
    match dir {
        Direction::Forward => {
            let (local_event_id, origin_server_ts) = events::table
                .filter(events::room_id.eq(room_id))
                .filter(events::origin_server_ts.ge(timestamp))
                .filter(events::is_outlier.eq(false))
                .filter(events::is_redacted.eq(false))
                .order_by((
                    events::origin_server_ts.asc(),
                    events::depth.asc(),
                    events::stream_ordering.asc(),
                ))
                .select((events::id, events::origin_server_ts))
                .first::<(OwnedEventId, UnixMillis)>(&mut connect().await?)
                .await?;
            Ok((local_event_id, origin_server_ts))
        }
        Direction::Backward => {
            let (local_event_id, origin_server_ts) = events::table
                .filter(events::room_id.eq(room_id))
                .filter(events::origin_server_ts.le(timestamp))
                .filter(events::is_outlier.eq(false))
                .filter(events::is_redacted.eq(false))
                .order_by((
                    events::origin_server_ts.desc(),
                    events::depth.desc(),
                    events::stream_ordering.desc(),
                ))
                .select((events::id, events::origin_server_ts))
                .first::<(OwnedEventId, UnixMillis)>(&mut connect().await?)
                .await?;
            Ok((local_event_id, origin_server_ts))
        }
    }
    // TODO: implement this function to find the event for a given timestamp
    // Check for gaps in the history where events could be hiding in between
    // the timestamp given and the event we were able to find locally
    // let is_event_next_to_backward_gap = false;
    // let is_event_next_to_forward_gap = false;
    // let local_event = None;
}

pub async fn get_event_sn_and_ty(event_id: &EventId) -> AppResult<(Seqnum, String)> {
    let (sn, ty) = events::table
        .find(event_id)
        .select((events::sn, events::ty))
        .first::<(Seqnum, String)>(&mut connect().await?)
        .await?;
    Ok((sn, ty))
}

pub async fn get_db_event(event_id: &EventId) -> AppResult<DbEvent> {
    events::table
        .find(event_id)
        .first::<DbEvent>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

pub async fn get_frame_id(room_id: &RoomId, event_sn: Seqnum) -> AppResult<i64> {
    event_points::table
        .filter(event_points::room_id.eq(room_id))
        .filter(event_points::event_sn.eq(event_sn))
        .select(event_points::frame_id)
        .first::<Option<i64>>(&mut connect().await?)
        .await?
        .ok_or(MatrixError::not_found("room frame id is not found").into())
}
pub async fn get_last_frame_id(room_id: &RoomId, before_sn: Option<Seqnum>) -> AppResult<i64> {
    if let Some(before_sn) = before_sn {
        event_points::table
            .filter(event_points::room_id.eq(room_id))
            .filter(event_points::event_sn.le(before_sn))
            .filter(event_points::frame_id.is_not_null())
            .select(event_points::frame_id)
            .order_by(event_points::event_sn.desc())
            .first::<Option<i64>>(&mut connect().await?)
            .await?
            .ok_or(MatrixError::not_found("room last frame id is not found").into())
    } else {
        event_points::table
            .filter(event_points::room_id.eq(room_id))
            .filter(event_points::frame_id.is_not_null())
            .select(event_points::frame_id)
            .order_by(event_points::event_sn.desc())
            .first::<Option<i64>>(&mut connect().await?)
            .await?
            .ok_or(MatrixError::not_found("room last frame id is not found").into())
    }
}
pub async fn update_frame_id(event_id: &EventId, frame_id: i64) -> AppResult<()> {
    diesel::update(event_points::table.find(event_id))
        .set(event_points::frame_id.eq(frame_id))
        .execute(&mut connect().await?)
        .await?;
    // diesel::update(events::table.find(event_id))
    //     .set(events::stream_ordering.eq(frame_id))
    //     .execute(&mut connect()?)?;
    Ok(())
}

pub async fn update_before_frame_id(event_id: &EventId, frame_id: i64) -> AppResult<()> {
    diesel::update(event_points::table.find(event_id))
        .set(event_points::before_frame_id.eq(frame_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

pub async fn update_frame_id_by_sn(event_sn: Seqnum, frame_id: i64) -> AppResult<()> {
    diesel::update(event_points::table.filter(event_points::event_sn.eq(event_sn)))
        .set(event_points::frame_id.eq(frame_id))
        .execute(&mut connect().await?)
        .await?;
    // diesel::update(events::table.filter(events::sn.eq(event_sn)))
    //     .set(events::stream_ordering.eq(frame_id))
    //     .execute(&mut connect()?)?;
    Ok(())
}

pub type PdusIterItem<'a> = (&'a Seqnum, &'a SnPduEvent);

pub fn parse_fetched_pdu(
    room_id: &RoomId,
    room_version: &RoomVersionId,
    raw_value: &RawJsonValue,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let value: CanonicalJsonObject = serde_json::from_str(raw_value.get()).map_err(|e| {
        warn!("error parsing fetched event {:?}: {:?}", raw_value, e);
        MatrixError::bad_json("invalid pdu in server response")
    })?;
    let parsed_room_id = value
        .get("room_id")
        .and_then(|id| RoomId::parse(id.as_str()?).ok());
    if let Some(parsed_room_id) = parsed_room_id
        && parsed_room_id != room_id
    {
        return Err(MatrixError::invalid_param("mismatched room_id in fetched pdu").into());
    }

    let event_id = match crate::event::gen_event_id(&value, room_version) {
        Ok(t) => t,
        Err(e) => {
            // Event could not be converted to canonical json
            error!(value = ?value, "error generating event id for fetched pdu: {:?}", e);
            return Err(MatrixError::bad_json("could not convert event to canonical json").into());
        }
    };
    check_create_event_for_room(room_id, room_version, &event_id, &value)?;
    Ok((event_id, value))
}

/// Rejects an `m.room.create` event that cannot be the create event of `room_id`, in room
/// versions whose room ID is derived from the create event (v12 onwards).
///
/// Such a create event must not carry a `room_id` of its own, and its reference hash must be
/// the room ID. The authorization rules cannot check the first part: by the time an event
/// reaches them its `room_id` has been filled in from the room it was received for, so the
/// field is checked here, on the event as received. Other events and earlier room versions
/// pass unchanged.
pub fn check_create_event_for_room(
    room_id: &RoomId,
    room_version: &RoomVersionId,
    event_id: &EventId,
    value: &CanonicalJsonObject,
) -> AppResult<()> {
    if value.get("type").and_then(|t| t.as_str()) != Some("m.room.create") {
        return Ok(());
    }
    let version_rules = crate::room::get_version_rules(room_version)?;
    if version_rules.room_id_format != RoomIdFormatVersion::V2 {
        return Ok(());
    }
    if value.contains_key("room_id") {
        return Err(MatrixError::bad_json(
            "m.room.create event must not have a room_id in this room version",
        )
        .into());
    }
    if RoomId::new_v2(event_id.localpart()).ok().as_deref() != Some(room_id) {
        return Err(MatrixError::bad_json(format!(
            "m.room.create event {event_id} is not the create event of room {room_id}"
        ))
        .into());
    }
    Ok(())
}

pub async fn parse_incoming_pdu(
    raw_value: &RawJsonValue,
) -> AppResult<(
    OwnedEventId,
    CanonicalJsonObject,
    OwnedRoomId,
    RoomVersionId,
)> {
    let value: CanonicalJsonObject = serde_json::from_str(raw_value.get()).map_err(|e| {
        warn!("error parsing incoming event {:?}: {:?}", raw_value, e);
        MatrixError::bad_json("invalid pdu in server response")
    })?;
    let room_id = value
        .get("room_id")
        .and_then(|id| RoomId::parse(id.as_str()?).ok())
        .ok_or(MatrixError::invalid_param("invalid room id in pdu"))?;

    let room_version_id = crate::room::get_version(&room_id).await.map_err(|_| {
        MatrixError::invalid_param(format!(
            "server is not in room `{room_id}` when parse incoming event"
        ))
    })?;

    let event_id = match crate::event::gen_event_id(&value, &room_version_id) {
        Ok(t) => t,
        Err(_) => {
            // Event could not be converted to canonical json
            return Err(
                MatrixError::invalid_param("could not convert event to canonical json").into(),
            );
        }
    };
    Ok((event_id, value, room_id, room_version_id))
}

pub async fn seen_event_ids(
    room_id: &RoomId,
    event_ids: &[OwnedEventId],
) -> AppResult<Vec<OwnedEventId>> {
    let seen_events = events::table
        .filter(events::room_id.eq(room_id))
        .filter(events::id.eq_any(event_ids))
        .select(events::id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await?;
    Ok(seen_events)
}
#[inline]
pub async fn ignored_filter(item: PdusIterItem<'_>, user_id: &UserId) -> bool {
    let (_, pdu) = item;
    !is_ignored_pdu(pdu, user_id).await
}

#[inline]
pub fn ignored_filter_with_ignored_users(
    item: PdusIterItem,
    ignored_users: &BTreeSet<OwnedUserId>,
) -> bool {
    let (_, pdu) = item;
    !is_ignored_pdu_by_ignored_users(pdu, ignored_users)
}

pub async fn is_ignored_pdu(pdu: &SnPduEvent, user_id: &UserId) -> bool {
    let ignored_users = crate::user::ignored_users(user_id).await;
    is_ignored_pdu_by_ignored_users(pdu, &ignored_users)
}

pub fn is_ignored_pdu_by_ignored_users(
    pdu: &SnPduEvent,
    ignored_users: &BTreeSet<OwnedUserId>,
) -> bool {
    // exclude Synapse's dummy events from bloating up response bodies. clients
    // don't need to see this.
    if pdu.event_ty.to_string() == "org.matrix.dummy_event" {
        return true;
    }

    is_ignored_sender_pdu_by_ignored_users(pdu, ignored_users)
}

pub async fn is_ignored_sender_pdu(pdu: &SnPduEvent, user_id: &UserId) -> bool {
    let ignored_users = crate::user::ignored_users(user_id).await;
    is_ignored_sender_pdu_by_ignored_users(pdu, &ignored_users)
}

pub fn is_ignored_sender_pdu_by_ignored_users(
    pdu: &SnPduEvent,
    ignored_users: &BTreeSet<OwnedUserId>,
) -> bool {
    pdu.state_key.is_none() && ignored_users.contains(&pdu.sender)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn v12_create() -> CanonicalJsonObject {
        serde_json::from_value(json!({
            "auth_events": [],
            "content": { "room_version": "12" },
            "depth": 1,
            "hashes": { "sha256": "hash" },
            "origin_server_ts": 1,
            "prev_events": [],
            "sender": "@alice:example.org",
            "signatures": { "example.org": { "ed25519:key": "sig" } },
            "state_key": "",
            "type": "m.room.create"
        }))
        .unwrap()
    }

    #[test]
    fn v12_create_event_must_be_the_rooms_own_and_have_no_room_id() {
        let create = v12_create();
        let event_id = gen_event_id(&create, &RoomVersionId::V12).unwrap();
        let room_id = RoomId::new_v2(event_id.localpart()).unwrap();
        check_create_event_for_room(&room_id, &RoomVersionId::V12, &event_id, &create).unwrap();

        // The create event of some other room.
        let other_room = RoomId::new_v2("other").unwrap();
        assert!(
            check_create_event_for_room(&other_room, &RoomVersionId::V12, &event_id, &create)
                .is_err()
        );

        // A `room_id` field is rejected outright, whatever it names.
        let mut with_room_id = create.clone();
        with_room_id.insert("room_id".to_owned(), room_id.as_str().into());
        let with_room_id_event_id = gen_event_id(&with_room_id, &RoomVersionId::V12).unwrap();
        assert!(
            check_create_event_for_room(
                &room_id,
                &RoomVersionId::V12,
                &with_room_id_event_id,
                &with_room_id,
            )
            .is_err()
        );
    }

    #[test]
    fn create_event_check_leaves_other_events_and_versions_alone() {
        let room_id = RoomId::parse("!room:example.org").unwrap();
        let mut create = v12_create();
        create.insert("room_id".to_owned(), room_id.as_str().into());
        let event_id = gen_event_id(&create, &RoomVersionId::V11).unwrap();
        check_create_event_for_room(&room_id, &RoomVersionId::V11, &event_id, &create).unwrap();

        let mut member = create.clone();
        member.insert("type".to_owned(), "m.room.member".into());
        member.insert("state_key".to_owned(), "@alice:example.org".into());
        let event_id = gen_event_id(&member, &RoomVersionId::V12).unwrap();
        check_create_event_for_room(&room_id, &RoomVersionId::V12, &event_id, &member).unwrap();
    }
}
