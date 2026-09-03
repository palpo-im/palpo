use std::ops::{Deref, DerefMut};

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::events::TimelineEventType;
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonObject, RawJsonValue};
use crate::core::state::{Event, StateError};
use crate::core::{self, Seqnum, UnixMillis};
use crate::data::room::{DbEventData, NewDbEvent};
use crate::data::schema::*;
use crate::data::{connect, diesel_exists};
use crate::event::fetching::{
    fetch_and_process_auth_chain, fetch_and_process_missing_events,
    fetch_and_process_missing_state, fetch_and_process_missing_state_by_ids,
};
use crate::event::handler::auth_check;
use crate::event::resolver::resolve_state_at_incoming;
use crate::event::{PduEvent, SnPduEvent, ensure_event_sn};
use crate::room::state::update_backward_extremities;
use crate::room::timeline;
use crate::utils::SeqnumQueueGuard;
use crate::{AppError, AppResult, MatrixError};

#[derive(Clone, Debug)]
pub struct OutlierPdu {
    pub pdu: PduEvent,
    pub json_data: CanonicalJsonObject,
    pub soft_failed: bool,
    /// The room's Policy Server (MSC4284) refused to vouch for this event.
    ///
    /// Kept apart from `soft_failed` because the DAG-recovery paths clear that flag once
    /// the event turns out to be well-formed and authorised, which a policy refusal has
    /// nothing to do with. It is folded in when the event is persisted, so no recovery
    /// path can promote a refused event to the timeline.
    pub policy_refused: bool,

    pub remote_server: OwnedServerName,
    pub room_id: OwnedRoomId,
    pub room_version: RoomVersionId,
    pub event_sn: Option<Seqnum>,
}

pub(crate) const POLICY_REFUSED_REASON: &str = "event refused by the room policy server";

fn rejection_reason_for_storage(
    rejection_reason: Option<String>,
    policy_refused: bool,
) -> Option<String> {
    rejection_reason.or_else(|| policy_refused.then(|| POLICY_REFUSED_REASON.to_owned()))
}

impl AsRef<PduEvent> for OutlierPdu {
    fn as_ref(&self) -> &PduEvent {
        &self.pdu
    }
}
impl AsMut<PduEvent> for OutlierPdu {
    fn as_mut(&mut self) -> &mut PduEvent {
        &mut self.pdu
    }
}
impl DerefMut for OutlierPdu {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pdu
    }
}
impl Deref for OutlierPdu {
    type Target = PduEvent;

    fn deref(&self) -> &Self::Target {
        &self.pdu
    }
}

impl crate::core::state::Event for OutlierPdu {
    type Id = OwnedEventId;

    fn event_id(&self) -> &Self::Id {
        &self.event_id
    }

    fn room_id(&self) -> &RoomId {
        &self.room_id
    }

    fn sender(&self) -> &UserId {
        &self.sender
    }

    fn event_type(&self) -> &TimelineEventType {
        &self.event_ty
    }

    fn content(&self) -> &RawJsonValue {
        &self.content
    }

    fn origin_server_ts(&self) -> UnixMillis {
        self.origin_server_ts
    }

    fn state_key(&self) -> Option<&str> {
        self.state_key.as_deref()
    }

    fn prev_events(&self) -> &[Self::Id] {
        self.prev_events.deref()
    }

    fn auth_events(&self) -> &[Self::Id] {
        self.auth_events.deref()
    }

    fn redacts(&self) -> Option<&Self::Id> {
        self.redacts.as_ref()
    }

    fn rejected(&self) -> bool {
        self.pdu.rejected()
    }
}

impl OutlierPdu {
    pub async fn save_to_database(
        self,
        is_backfill: bool,
    ) -> AppResult<(SnPduEvent, CanonicalJsonObject, Option<SeqnumQueueGuard>)> {
        let Self {
            mut pdu,
            json_data,
            soft_failed,
            policy_refused,
            room_id,
            event_sn,
            ..
        } = self;
        let soft_failed = soft_failed || policy_refused;
        pdu.rejection_reason = rejection_reason_for_storage(pdu.rejection_reason, policy_refused);
        if let Some(event_sn) = event_sn {
            if policy_refused {
                // Existing outliers may be checked again after their auth arrives.
                // Persist the refusal as well as carrying it on the returned PDU.
                diesel::update(events::table.filter(events::id.eq(&pdu.event_id)))
                    .set((
                        events::is_rejected.eq(true),
                        events::soft_failed.eq(true),
                        events::rejection_reason.eq(&pdu.rejection_reason),
                    ))
                    .execute(&mut connect().await?)
                    .await?;
            }
            return Ok((
                SnPduEvent {
                    pdu,
                    event_sn,
                    is_outlier: true,
                    soft_failed,
                    is_backfill,
                },
                json_data,
                None,
            ));
        }
        let (event_sn, event_guard) = ensure_event_sn(&room_id, &pdu.event_id).await?;
        let mut db_event = NewDbEvent::from_canonical_json_with_room_id(
            &pdu.event_id,
            event_sn,
            &json_data,
            is_backfill,
            &room_id,
        )?;
        db_event.is_outlier = true;
        db_event.soft_failed = soft_failed;
        db_event.is_rejected = pdu.rejected();
        db_event.rejection_reason = pdu.rejection_reason.clone();
        let event_data = DbEventData {
            event_id: pdu.event_id.clone(),
            event_sn,
            room_id: pdu.room_id.clone(),
            internal_metadata: None,
            json_data: serde_json::to_value(&json_data)?,
            format_version: None,
        };
        // An outlier becomes queryable as soon as its JSON row commits. Keep metadata and
        // JSON together so feature-specific outlier indexes can share this transaction.
        let (is_rejected, rejection_reason) = connect()
            .await?
            .transaction::<_, AppError, _>(async |conn| {
                // Create the row before locking it so the lock below always has one to
                // take. A concurrent replay of the same event serialises against this
                // transaction either here, while the row is still uncommitted, or on the
                // lock once it is, so neither can merge onto a verdict it cannot see.
                diesel::insert_into(events::table)
                    .values(&db_event)
                    .on_conflict_do_nothing()
                    .execute(conn)
                    .await?;
                // A stored rejection is durable and a replay must never lift one. Auth
                // rejection follows from the event's immutable auth references, and an
                // MSC4284 refusal stays refused. A replay, however, carries no verdict of
                // its own whenever the DAG is momentarily incomplete: it leaves the event
                // unauthorised in `process_to_outlier_pdu`, which deliberately skips the
                // Policy Server request and so yields `policy_refused == false`. Letting
                // that overwrite the stored row would clear `is_rejected`, the only column
                // `timeline::stream` filters on, and hand a refused event to clients.
                let (was_rejected, stored_reason) = events::table
                    .find(&db_event.id)
                    .select((events::is_rejected, events::rejection_reason))
                    .for_update()
                    .first::<(bool, Option<String>)>(conn)
                    .await?;
                db_event.is_rejected |= was_rejected;
                // The first verdict wins; a later one can only supply a missing reason.
                db_event.rejection_reason = stored_reason.or(db_event.rejection_reason.take());
                // Both explicit rejection writers pair these columns, and the returned PDU
                // below reports the same. Keep the stored row from disagreeing with either.
                db_event.soft_failed |= db_event.is_rejected;
                db_event.save_with_conn(conn).await?;
                event_data.save_with_conn(conn).await?;
                Ok((db_event.is_rejected, db_event.rejection_reason.clone()))
            })
            .await?;
        // Report what was persisted rather than what this replay believed: a caller handed
        // a promoted PDU would append it to the timeline and undo the merge above.
        pdu.rejection_reason = rejection_reason;
        let soft_failed = soft_failed || is_rejected;
        let pdu = SnPduEvent {
            pdu,
            event_sn,
            is_outlier: true,
            soft_failed,
            is_backfill,
        };
        update_backward_extremities(&pdu).await?;
        Ok((pdu, json_data, event_guard))
    }

    pub async fn process_incoming(
        mut self,
        remote_server: &ServerName,
        is_backfill: bool,
    ) -> AppResult<(SnPduEvent, CanonicalJsonObject, Option<SeqnumQueueGuard>)> {
        // A rejected event cannot become valid by fetching more predecessors:
        // event IDs and their auth references are immutable. Persist it without
        // issuing federation requests so later valid descendants can reconnect
        // to the last accepted state.
        if self.policy_refused || !self.soft_failed || self.rejected() {
            return self.save_to_database(is_backfill).await;
        }

        // Fetch any missing prev events doing all checks listed here starting at 1. These are
        // timeline events
        if let Err(e) = fetch_and_process_missing_events(
            &self.remote_server,
            &self.room_id,
            &self.room_version,
            &self,
            is_backfill,
        )
        .await
        {
            if let AppError::Matrix(MatrixError { ref kind, .. }) = e {
                if *kind == core::error::ErrorKind::BadJson {
                    self.rejection_reason = Some(format!("bad prev events: {}", e));
                    let _state_lock = crate::room::lock_state(&self.room_id).await;
                    return self.save_to_database(is_backfill).await;
                } else {
                    self.soft_failed = true;
                }
            } else {
                self.soft_failed = true;
            }
        }

        self.process_pulled(remote_server, is_backfill).await
    }

    async fn any_auth_event_rejected(&self) -> AppResult<bool> {
        let query = events::table
            .filter(events::id.eq_any(&self.pdu.auth_events))
            .filter(events::is_rejected.eq(true));
        Ok(diesel_exists!(query, &mut connect().await?)?)
    }
    pub async fn process_pulled(
        mut self,
        _remote_server: &ServerName,
        is_backfill: bool,
    ) -> AppResult<(SnPduEvent, CanonicalJsonObject, Option<SeqnumQueueGuard>)> {
        let version_rules = crate::room::get_version_rules(&self.room_version)?;

        if self.policy_refused || !self.soft_failed || self.rejected() {
            return self.save_to_database(is_backfill).await;
        }

        if self.any_auth_event_rejected().await?
            && let Err(e) = fetch_and_process_auth_chain(
                &self.remote_server,
                &self.room_id,
                &self.room_version,
                &self.pdu.event_id,
            )
            .await
        {
            if let AppError::HttpStatus(_) = e {
                self.soft_failed = true;
            } else {
                self.rejection_reason = Some("one or more auth events are rejected".to_string());
            }
            return self.save_to_database(is_backfill).await;
        }
        let (_prev_events, missing_prev_event_ids) =
            timeline::get_may_missing_pdus(&self.room_id, &self.pdu.prev_events).await?;

        if !missing_prev_event_ids.is_empty() {
            for event_id in &missing_prev_event_ids {
                let missing_events = match fetch_and_process_missing_state_by_ids(
                    &self.remote_server,
                    &self.room_id,
                    &self.room_version,
                    event_id,
                )
                .await
                {
                    Ok(missing_events) => {
                        self.soft_failed = !missing_events.is_empty();
                        missing_events
                    }
                    Err(e) => {
                        if let AppError::Matrix(MatrixError { ref kind, .. }) = e {
                            if *kind == core::error::ErrorKind::BadJson {
                                self.rejection_reason =
                                    Some(format!("failed to bad prev events: {}", e));
                            } else {
                                self.soft_failed = true;
                            }
                        } else {
                            self.soft_failed = true;
                        }
                        vec![]
                    }
                };
                if !missing_events.is_empty() {
                    for event_id in &missing_events {
                        if let Err(e) = fetch_and_process_auth_chain(
                            &self.remote_server,
                            &self.room_id,
                            &self.room_version,
                            event_id,
                        )
                        .await
                        {
                            warn!("error fetching auth chain for {}: {}", event_id, e);
                        }
                    }
                }
            }
        }

        if self.pdu.rejection_reason.is_none() {
            let state_at_incoming_event = if let Some(state_at_incoming_event) =
                resolve_state_at_incoming(&self.pdu, &version_rules).await?
            {
                Some(state_at_incoming_event)
            } else {
                if missing_prev_event_ids.is_empty() {
                    fetch_and_process_missing_state(
                        &self.remote_server,
                        &self.room_id,
                        &self.room_version,
                        &self.pdu.event_id,
                    )
                    .await
                    .ok()
                    .map(|r| r.state_events)
                } else {
                    None
                }
            };
            if let Err(e) =
                auth_check(&self.pdu, &version_rules, state_at_incoming_event.as_ref()).await
            {
                match e {
                    AppError::State(
                        StateError::Forbidden(brief) | StateError::AuthEvent(brief),
                    ) => {
                        self.pdu.rejection_reason = Some(brief);
                    }
                    _ => {
                        self.soft_failed = true;
                    }
                }
            } else {
                self.soft_failed = false;
                self.policy_refused = !crate::room::policy::is_event_allowed(
                    &self.room_id,
                    &mut self.json_data,
                    &version_rules,
                )
                .await;
            }
        }
        self.save_to_database(is_backfill).await
    }
}

#[cfg(test)]
mod tests {
    use super::{POLICY_REFUSED_REASON, rejection_reason_for_storage};

    #[test]
    fn policy_refusal_is_persisted_as_a_rejection() {
        assert_eq!(
            rejection_reason_for_storage(None, true).as_deref(),
            Some(POLICY_REFUSED_REASON)
        );
        assert_eq!(
            rejection_reason_for_storage(Some("auth rejected".to_owned()), true).as_deref(),
            Some("auth rejected")
        );
        assert_eq!(rejection_reason_for_storage(None, false), None);
    }
}
