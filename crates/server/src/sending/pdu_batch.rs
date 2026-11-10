//! Request-local PDU reads and room-version reuse for a federation transaction.
use std::collections::{BTreeSet, HashMap};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::serde::{CanonicalJsonObject, RawJsonValue};
use crate::core::{OwnedEventId, OwnedRoomId, RoomVersionId, ServerName};
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppError, AppResult, room};

type Versions = HashMap<OwnedRoomId, Option<RoomVersionId>>;

fn json_room(json: &CanonicalJsonObject) -> Option<OwnedRoomId> {
    json.get("room_id")?.as_str()?.try_into().ok()
}

fn json_event(json: &CanonicalJsonObject) -> Option<OwnedEventId> {
    json.get("event_id")?.as_str()?.try_into().ok()
}

async fn load_versions(room_ids: &BTreeSet<OwnedRoomId>, versions: &mut Versions) {
    let ids = room_ids
        .iter()
        .filter(|id| !versions.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    for ids in ids.chunks(1_000) {
        let rows = async {
            rooms::table
                .filter(rooms::id.eq_any(ids))
                .select((rooms::id, rooms::version))
                .load::<(OwnedRoomId, String)>(&mut connect().await?)
                .await
                .map_err(AppError::from)
        }
        .await;
        if let Ok(rows) = rows {
            versions.extend(
                rows.into_iter()
                    .map(|(room, version)| (room, RoomVersionId::try_from(version.as_str()).ok())),
            );
        }
        // Preserve get_version's create-state fallback for rooms without a
        // metadata row, and its best-effort behavior on a query failure. Failed
        // lookups are reused only inside this transaction, never globally.
        for room in ids {
            if !versions.contains_key(room) {
                versions.insert(room.clone(), room::get_version(room).await.ok());
            }
        }
    }
}

pub(super) async fn prepare(
    server: &ServerName,
    event_ids: &[OwnedEventId],
) -> AppResult<Vec<Box<RawJsonValue>>> {
    let stored = room::timeline::load_pdu_json_batch(event_ids).await?;
    let mut jsons = Vec::with_capacity(event_ids.len());
    for id in event_ids {
        // Keep duplicate inputs, ordering, lazy canonical/retention checks and
        // the first missing-event error of the original sequential send loop.
        let json = stored.get(id).await?.ok_or_else(|| {
            tracing::error!("event not found: {server} {id:?}");
            AppError::internal(format!("event not found: {server} {id}"))
        })?;
        jsons.push(json);
    }
    let mut versions = Versions::new();
    load_versions(&jsons.iter().filter_map(json_room).collect(), &mut versions).await;

    // The existing converter falls back by the JSON's event_id, not by the
    // queue ID or event_datas.room_id. Retain that distinction for legacy and
    // v12 JSON, including inconsistent/unavailable metadata.
    let fallback_ids = jsons
        .iter()
        .filter(|json| {
            json_room(json)
                .and_then(|room| versions.get(&room))
                .and_then(Option::as_ref)
                .is_none()
        })
        .filter_map(json_event)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut event_rooms = HashMap::new();
    for ids in fallback_ids.chunks(1_000) {
        let rows = async {
            events::table
                .filter(events::id.eq_any(ids))
                .select((events::id, events::room_id))
                .load::<(OwnedEventId, OwnedRoomId)>(&mut connect().await?)
                .await
                .map_err(AppError::from)
        }
        .await;
        // Version lookup failures were non-fatal in the individual converter.
        if let Ok(rows) = rows {
            event_rooms.extend(rows);
        }
    }
    load_versions(&event_rooms.values().cloned().collect(), &mut versions).await;
    Ok(jsons
        .into_iter()
        .map(|json| {
            let version = json_room(&json)
                .and_then(|room| versions.get(&room))
                .and_then(Option::as_ref)
                .or_else(|| {
                    json_event(&json)
                        .and_then(|id| event_rooms.get(&id))
                        .and_then(|room| versions.get(room))
                        .and_then(Option::as_ref)
                });
            super::federation_event_with_version(json, version)
        })
        .collect())
}
