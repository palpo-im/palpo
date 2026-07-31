//! Sticky events ([MSC4354]).
//!
//! A sticky event is an ordinary timeline event that additionally must reach every joined
//! client, regardless of the `timeline_limit` a sync used, until it expires. Palpo tracks
//! the derived expiry instant in `event_stickies` so that a sync can find a room's
//! unexpired sticky events without reading event JSON, and so that expiry survives a
//! restart: the absolute instant is computed once, when the event is persisted.
//!
//! [MSC4354]: https://github.com/matrix-org/matrix-spec-proposals/pull/4354

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::identifiers::*;
use crate::core::{Seqnum, UnixMillis};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::{PduEvent, STICKY_TTL_KEY};
use crate::{AppResult, SnPduEvent};

/// A sticky event that is still within its sticky window.
#[derive(Debug, Clone)]
pub struct StickyEntry {
    pub event_id: OwnedEventId,
    pub event_sn: Seqnum,
    pub expires_at: UnixMillis,
}

/// Records the event's sticky window, if it has one.
///
/// Called when the event is first persisted, not when it is promoted to the timeline. A
/// federated event can sit as an outlier for a while before its DAG is filled in, and
/// measuring the window from the promotion instead of the receipt would hand a sender with
/// a clock set in the future the whole outlier-processing delay as extra stickiness --
/// exactly the skew `sticky_expires_at` exists to bound.
///
/// Events without a valid `msc4354_sticky` object are ordinary events and are not recorded.
/// Neither is one that is already expired on arrival -- an old sticky event coming in over
/// federation -- since it can never be delivered.
///
/// `do_nothing` on conflict is what keeps the first receipt authoritative if the same event
/// is stored again.
pub async fn record(pdu: &PduEvent, event_sn: Seqnum, received_at: UnixMillis) -> AppResult<()> {
    let Some(expires_at) = pdu.sticky_expires_at(received_at) else {
        return Ok(());
    };
    if expires_at <= UnixMillis::now() {
        return Ok(());
    }

    diesel::insert_into(event_stickies::table)
        .values((
            event_stickies::event_id.eq(&pdu.event_id),
            event_stickies::event_sn.eq(event_sn),
            event_stickies::room_id.eq(&pdu.room_id),
            event_stickies::expires_at.eq(expires_at.0 as i64),
        ))
        .on_conflict(event_stickies::event_id)
        .do_nothing()
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// The room's sticky events that have not expired at `now`.
///
/// `since_sn` restricts the result to events the client has not been told about yet, giving
/// the stream-like delivery MSC4354 asks for: a client sees each sticky event once. Passing
/// `None` -- an initial sync, or a room the user has just joined -- returns every unexpired
/// sticky event in the room.
pub async fn unexpired(
    room_id: &RoomId,
    since_sn: Option<Seqnum>,
    until_sn: Seqnum,
    now: UnixMillis,
) -> AppResult<Vec<StickyEntry>> {
    // Recorded on first storage, so an event that never made it out of the outlier stage,
    // was soft failed, or was rejected still has a row here and must not be delivered.
    let deliverable = events::table
        .filter(events::is_outlier.eq(false))
        .filter(events::soft_failed.eq(false))
        .filter(events::is_rejected.eq(false))
        .select(events::id);

    // Sync positions are half-open: the `since` token is the first position a client has
    // not seen, and the token handed back is the first it will not see this time.
    let mut query = event_stickies::table
        .filter(event_stickies::room_id.eq(room_id))
        .filter(event_stickies::expires_at.gt(now.0 as i64))
        .filter(event_stickies::event_sn.lt(until_sn))
        .filter(event_stickies::event_id.eq_any(deliverable))
        .into_boxed();

    if let Some(since_sn) = since_sn {
        query = query.filter(event_stickies::event_sn.ge(since_sn));
    }

    let rows = query
        .order(event_stickies::event_sn.asc())
        .select((
            event_stickies::event_id,
            event_stickies::event_sn,
            event_stickies::expires_at,
        ))
        .load::<(OwnedEventId, Seqnum, i64)>(&mut connect().await?)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(event_id, event_sn, expires_at)| StickyEntry {
            event_id,
            event_sn,
            expires_at: UnixMillis(expires_at as u64),
        })
        .collect())
}

/// Deletes rows for events that can no longer be delivered.
///
/// Purely a space reclaim: reads already filter on `expires_at`, so a row that outlives its
/// event's stickiness is never delivered.
pub async fn delete_expired(now: UnixMillis) -> AppResult<usize> {
    let deleted =
        diesel::delete(event_stickies::table.filter(event_stickies::expires_at.le(now.0 as i64)))
            .execute(&mut connect().await?)
            .await?;
    Ok(deleted)
}

/// How much of an event's sticky window is left, clamped at zero.
pub fn ttl_ms(expires_at: UnixMillis, now: UnixMillis) -> u64 {
    expires_at.0.saturating_sub(now.0)
}

/// Annotates a sticky event with how long it has left, for delivery in `/sync`.
///
/// Clients use `unsigned.msc4354_sticky_duration_ttl_ms` instead of computing the remaining
/// time from `origin_server_ts` themselves, which keeps a client whose clock is wrong from
/// expiring the event at the wrong moment.
pub fn with_ttl(mut pdu: SnPduEvent, expires_at: UnixMillis, now: UnixMillis) -> SnPduEvent {
    pdu.pdu.unsigned.insert(
        STICKY_TTL_KEY.to_owned(),
        serde_json::value::to_raw_value(&ttl_ms(expires_at, now)).expect("u64 is valid json"),
    );
    pdu
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ttl_ms;
    use crate::core::UnixMillis;
    use crate::core::events::TimelineEventType;
    use crate::core::identifiers::*;
    use crate::core::serde::JsonValue;
    use crate::event::{EventHash, PduEvent, STICKY_KEY};

    fn pdu(origin_server_ts: u64, sticky: Option<JsonValue>) -> PduEvent {
        let mut extra_data = std::collections::BTreeMap::new();
        if let Some(sticky) = sticky {
            extra_data.insert(STICKY_KEY.to_owned(), sticky);
        }
        PduEvent {
            event_id: EventId::parse("$event:example.org").unwrap().to_owned(),
            event_ty: TimelineEventType::RoomMessage,
            room_id: RoomId::parse("!room:example.org").unwrap().to_owned(),
            sender: UserId::parse("@alice:example.org").unwrap().to_owned(),
            origin_server_ts: UnixMillis(origin_server_ts),
            content: serde_json::from_str("{}").unwrap(),
            state_key: None,
            prev_events: vec![],
            depth: 1,
            auth_events: vec![],
            redacts: None,
            unsigned: Default::default(),
            hashes: EventHash {
                sha256: String::new(),
            },
            signatures: None,
            extra_data,
            rejection_reason: None,
        }
    }

    #[test]
    fn only_well_formed_durations_make_an_event_sticky() {
        assert_eq!(
            pdu(0, Some(json!({ "duration_ms": 60_000 })))
                .sticky_duration_ms()
                .map(|d| d.get()),
            Some(60_000)
        );
        // An hour is the maximum the MSC allows.
        assert_eq!(
            pdu(0, Some(json!({ "duration_ms": 3_600_000 })))
                .sticky_duration_ms()
                .map(|d| d.get()),
            Some(3_600_000)
        );

        // Anything malformed leaves the event ordinary rather than rejecting it, so a peer
        // cannot get an event dropped by attaching nonsense to it.
        for sticky in [
            json!({ "duration_ms": 3_600_001 }),
            json!({ "duration_ms": -1 }),
            json!({ "duration_ms": 1000.5 }),
            json!({ "duration_ms": "1000" }),
            json!({}),
            json!("sticky"),
        ] {
            assert_eq!(pdu(0, Some(sticky.clone())).sticky_duration_ms(), None);
        }

        assert_eq!(pdu(0, None).sticky_duration_ms(), None);
    }

    #[test]
    fn sticky_window_starts_at_the_earlier_of_receipt_and_origin() {
        let sticky = Some(json!({ "duration_ms": 60_000 }));

        // A sender whose clock runs fast cannot extend its own stickiness: the window is
        // measured from when we received the event.
        let future = pdu(9_000_000, sticky.clone());
        assert_eq!(
            future.sticky_expires_at(UnixMillis(1_000_000)),
            Some(UnixMillis(1_060_000))
        );

        // An event that was sent a while ago expires that much sooner.
        let past = pdu(1_000_000, sticky);
        assert_eq!(
            past.sticky_expires_at(UnixMillis(1_030_000)),
            Some(UnixMillis(1_060_000))
        );

        assert_eq!(pdu(1_000_000, None).sticky_expires_at(UnixMillis(0)), None);
    }

    #[test]
    fn client_events_carry_the_sticky_object() {
        let sticky = pdu(1_000_000, Some(json!({ "duration_ms": 300_000 })));

        for serialized in [
            serde_json::to_value(sticky.to_sync_room_event()).unwrap(),
            serde_json::to_value(sticky.to_room_event()).unwrap(),
        ] {
            assert_eq!(serialized[STICKY_KEY], json!({ "duration_ms": 300_000 }));
        }

        // An out-of-range annotation is not a sticky event, so it is not echoed back to
        // clients as one.
        let invalid = pdu(1_000_000, Some(json!({ "duration_ms": 3_600_001 })));
        assert_eq!(
            serde_json::to_value(invalid.to_sync_room_event())
                .unwrap()
                .get(STICKY_KEY),
            None
        );

        // An ordinary event is untouched.
        let ordinary = pdu(1_000_000, None);
        assert_eq!(
            serde_json::to_value(ordinary.to_sync_room_event())
                .unwrap()
                .get(STICKY_KEY),
            None
        );
    }

    #[test]
    fn redaction_removes_the_stickiness() {
        let mut sticky = pdu(1_000_000, Some(json!({ "duration_ms": 300_000 })));
        let reason = pdu(1_000_001, None);

        sticky.redact(&reason).unwrap();

        // MSC4354 leaves the sticky object unprotected from redaction: a redacted sticky
        // event is an ordinary event.
        assert_eq!(sticky.sticky_duration_ms(), None);
        assert_eq!(sticky.sticky_expires_at(UnixMillis(1_000_000)), None);
    }

    #[test]
    fn ttl_counts_down_and_bottoms_out_at_zero() {
        let expires_at = UnixMillis(1_060_000);
        assert_eq!(ttl_ms(expires_at, UnixMillis(1_000_000)), 60_000);
        assert_eq!(ttl_ms(expires_at, UnixMillis(1_059_999)), 1);
        assert_eq!(ttl_ms(expires_at, UnixMillis(1_060_000)), 0);
        // Past the expiry the value stays at zero rather than wrapping.
        assert_eq!(ttl_ms(expires_at, UnixMillis(2_000_000)), 0);
    }
}
