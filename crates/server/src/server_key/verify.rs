use serde_json::value::RawValue as RawJsonValue;

use super::get_event_keys;
use crate::core::identifiers::*;
use crate::core::serde::canonical_json::{CanonicalJsonObject, CanonicalJsonValue};
use crate::core::signatures::{self, Verified};
use crate::event::gen_event_id_canonical_json;
use crate::{AppError, AppResult};

pub async fn validate_and_add_event_id(
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let (event_id, mut value) = gen_event_id_canonical_json(pdu, room_version)?;
    if let Err(e) = verify_event(&value, Some(room_version)).await {
        return Err(AppError::public(format!(
            "Event {event_id} failed verification: {e:?}"
        )));
    }

    value.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok((event_id, value))
}

pub async fn validate_and_add_event_id_no_fetch(
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
) -> AppResult<(OwnedEventId, CanonicalJsonObject)> {
    let (event_id, mut value) = gen_event_id_canonical_json(pdu, room_version)?;

    if let Err(e) = verify_event(&value, Some(room_version)).await {
        return Err(AppError::public(format!(
            "Event {event_id} failed verification: {e:?}"
        )));
    }

    value.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok((event_id, value))
}

pub async fn verify_event(
    event: &CanonicalJsonObject,
    room_version: Option<&RoomVersionId>,
) -> AppResult<Verified> {
    let room_version = room_version.unwrap_or(&RoomVersionId::V11);
    let room_rules = crate::room::get_rules(room_version)?;
    let keys = get_event_keys(event, &room_rules).await?;
    signatures::verify_event(&keys, event, &room_rules).map_err(Into::into)
}

pub async fn verify_json(
    event: &CanonicalJsonObject,
    room_version: Option<&RoomVersionId>,
) -> AppResult<()> {
    let room_version = room_version.unwrap_or(&RoomVersionId::V11);
    let room_rules = crate::room::get_rules(room_version)?;

    let keys = get_event_keys(event, &room_rules).await?;
    signatures::verify_json(&keys, event).map_err(Into::into)
}
