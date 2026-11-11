use std::collections::HashMap;

use diesel::prelude::*;
use horme::RunQueryDsl;

use crate::core::serde::JsonValue;
use crate::core::{EventId, OwnedEventId, OwnedRoomId, Seqnum};
use crate::data::schema::{event_datas, events};
use crate::{AppError, AppResult, PduEvent, SnPduEvent, data};

pub(super) const CHUNK_SIZE: usize = 256;

#[derive(Queryable, Selectable)]
#[diesel(table_name = event_datas)]
struct StoredPdu {
    event_id: OwnedEventId,
    event_sn: Seqnum,
    room_id: OwnedRoomId,
    json_data: JsonValue,
    #[diesel(select_expression = events::is_outlier)]
    is_outlier: bool,
    #[diesel(select_expression = events::soft_failed)]
    soft_failed: bool,
    #[diesel(select_expression = events::stream_ordering)]
    stream_ordering: i64,
    #[diesel(select_expression = events::rejection_reason)]
    rejection_reason: Option<String>,
}

impl StoredPdu {
    // Keep these checks at consumption time. In particular, retention can
    // scrub a snapshot whose event expired since the storage query.
    async fn into_pdu(self) -> AppResult<SnPduEvent> {
        let Self {
            event_id,
            event_sn,
            room_id,
            json_data,
            is_outlier,
            soft_failed,
            stream_ordering,
            rejection_reason,
        } = self;
        #[cfg(feature = "unstable-msc1763")]
        let mut json_data = json_data;
        #[cfg(feature = "unstable-msc1763")]
        crate::retention::sanitize_json(&event_id, &room_id, &mut json_data).await?;
        let mut pdu = PduEvent::from_json_value(&room_id, &event_id, json_data)
            .map_err(|_| AppError::internal("invalid pdu in db"))?;
        pdu.rejection_reason = rejection_reason;
        pdu.load_client_metadata().await?;
        Ok(SnPduEvent {
            pdu,
            event_sn,
            is_outlier,
            soft_failed,
            is_backfill: stream_ordering < 0,
        })
    }
}

pub(super) async fn load_one(event_id: &EventId) -> AppResult<SnPduEvent> {
    let row: StoredPdu = events::table
        .inner_join(event_datas::table.on(event_datas::event_id.eq(events::id)))
        .filter(events::id.eq(event_id))
        .select(StoredPdu::as_select())
        .first(&mut data::connect())
        .await?;
    row.into_pdu().await
}

pub(super) struct PduBatch(HashMap<OwnedEventId, StoredPdu>);

impl PduBatch {
    pub(super) async fn load<'a>(ids: impl Iterator<Item = &'a EventId>) -> AppResult<Self> {
        let ids: Vec<_> = ids.collect();
        if ids.is_empty() {
            return Ok(Self(HashMap::new()));
        }
        debug_assert!(ids.len() <= CHUNK_SIZE);
        let rows = events::table
            .inner_join(event_datas::table.on(event_datas::event_id.eq(events::id)))
            .filter(events::id.eq_any(ids))
            .select(StoredPdu::as_select())
            .load(&mut data::connect())
            .await?;
        Ok(Self(
            rows.into_iter()
                .map(|row: StoredPdu| (row.event_id.clone(), row))
                .collect(),
        ))
    }

    pub(super) async fn take(&mut self, event_id: &EventId) -> AppResult<SnPduEvent> {
        self.0
            .remove(event_id)
            .ok_or(diesel::result::Error::NotFound)?
            .into_pdu()
            .await
    }
}
