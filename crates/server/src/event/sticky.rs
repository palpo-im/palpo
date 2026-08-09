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
use diesel::result::Error as DieselError;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::{Seqnum, UnixMillis};
use crate::data::connect;
use crate::data::schema::*;
use crate::event::{PduEvent, STICKY_TTL_KEY};
use crate::{AppResult, SnPduEvent};

const STICKY_STREAM_LOCK_ID: i64 = 1_346_456_654;

async fn lock_sticky_stream(conn: &mut AsyncPgConnection) -> Result<(), DieselError> {
    diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
        .bind::<diesel::sql_types::BigInt, _>(STICKY_STREAM_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

/// Read the global stream position after all earlier sticky and device-inbox writes that
/// can affect this `/sync` have committed.
///
/// Both features allocate from `occur_sn_seq`. Taking their advisory locks in one
/// transaction is essential: two separate snapshots could let the second one observe a
/// sequence number allocated by an uncommitted writer that started after the first lock
/// was released, causing the returned sync token to skip that event permanently.
pub async fn curr_sn_after_sync_writes(
    user_id: &UserId,
    device_id: &DeviceId,
) -> AppResult<Seqnum> {
    let curr_sn = connect()
        .await?
        .transaction::<_, DieselError, _>(async |conn| {
            lock_sticky_stream(conn).await?;
            crate::data::user::device::lock_inbox_stream(conn, user_id, device_id).await?;
            diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT last_value FROM occur_sn_seq")
                .get_result::<Seqnum>(conn)
                .await
        })
        .await?;

    Ok(curr_sn)
}

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
/// On conflict, only the storage position is refreshed. The expiry is deliberately left
/// untouched so the first receipt remains authoritative if the same event is stored again.
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
        .do_update()
        .set((
            event_stickies::event_sn.eq(event_sn),
            event_stickies::room_id.eq(&pdu.room_id),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Promotes an event to the timeline and assigns its sticky sync position atomically.
///
/// A federated event can be stored as an outlier and promoted much later. By then clients
/// have synced past the position it was given on arrival, so delivering it there would
/// mean never delivering it. Taking a fresh position at promotion puts it back in front of
/// every client, which is what the delivery guarantee requires.
///
/// The expiry still runs from first receipt. Promotion also recreates a missing sticky row
/// from the receipt time stored with the event. This makes a retry self-healing if the
/// process failed after persisting the event but before `record` completed. For sticky
/// events the row, `is_outlier` transition, and delivery position share one transaction,
/// so a crash or database error cannot publish the timeline event without making its
/// sticky copy deliverable. The advisory transaction lock also prevents a sync on another
/// node from publishing a token that includes the new sequence before the row update
/// commits.
pub async fn promote_to_timeline(pdu: &SnPduEvent) -> AppResult<()> {
    // Ordinary events stay on the existing hot path: no sticky-table query, transaction,
    // advisory lock, or extra sequence allocation.
    if pdu.sticky_duration_ms().is_none() {
        diesel::update(events::table.find(&*pdu.event_id))
            .set(events::is_outlier.eq(false))
            .execute(&mut connect().await?)
            .await?;
        return Ok(());
    }

    connect()
        .await?
        .transaction::<_, DieselError, _>(async |conn| {
            lock_sticky_stream(conn).await?;

            // `received_at` is persisted with newly stored events. The fallback covers
            // events created by older code and direct membership paths which did not
            // persist it; those reach promotion immediately, so `now` is their receipt
            // time for practical purposes.
            // Lock the event row so a concurrent redaction and promotion have a defined
            // order. Without this, promotion could read the old unredacted PDU, race with
            // `redact_pdu` deleting its sticky row, and recreate that row after the
            // redaction committed.
            let (stored_received_at, is_redacted) = events::table
                .find(&*pdu.event_id)
                .select((events::received_at, events::is_redacted))
                .for_update()
                .first::<(Option<i64>, bool)>(conn)
                .await?;
            let received_at = stored_received_at
                .and_then(|value| u64::try_from(value).ok())
                .map(UnixMillis)
                .unwrap_or_else(UnixMillis::now);
            let now = UnixMillis::now();
            if is_redacted {
                // Also repairs a stale row left by an older server or interrupted cleanup.
                diesel::delete(
                    event_stickies::table.filter(event_stickies::event_id.eq(&pdu.event_id)),
                )
                .execute(conn)
                .await?;
            } else if let Some(expires_at) = pdu.sticky_expires_at(received_at)
                && expires_at > now
            {
                diesel::insert_into(event_stickies::table)
                    .values((
                        event_stickies::event_id.eq(&pdu.event_id),
                        event_stickies::event_sn.eq(pdu.event_sn),
                        event_stickies::room_id.eq(&pdu.room_id),
                        event_stickies::expires_at.eq(expires_at.0 as i64),
                    ))
                    .on_conflict(event_stickies::event_id)
                    .do_update()
                    .set((
                        event_stickies::event_sn.eq(pdu.event_sn),
                        event_stickies::room_id.eq(&pdu.room_id),
                    ))
                    .execute(conn)
                    .await?;
            }

            // `nextval` is evaluated only for a pending, unexpired row. An event that
            // expired while it was an outlier consumes no delivery position.
            if !is_redacted {
                diesel::update(
                    event_stickies::table
                        .filter(event_stickies::event_id.eq(&pdu.event_id))
                        .filter(event_stickies::deliver_sn.is_null())
                        .filter(event_stickies::expires_at.gt(now.0 as i64)),
                )
                .set(event_stickies::deliver_sn.eq(diesel::dsl::sql::<
                    diesel::sql_types::Nullable<diesel::sql_types::BigInt>,
                >("nextval('occur_sn_seq')")))
                .execute(conn)
                .await?;
            }

            diesel::update(events::table.find(&*pdu.event_id))
                .set(events::is_outlier.eq(false))
                .execute(conn)
                .await?;
            Ok(())
        })
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
    // Rows are written on first storage, so an event still sitting as an outlier -- or one
    // that was soft failed or rejected -- has a row but no delivery position, and must not
    // be delivered.
    //
    // Sync positions are half-open: the `since` token is the first position a client has
    // not seen, and the token handed back is the first it will not see this time.
    let mut query = event_stickies::table
        .filter(event_stickies::room_id.eq(room_id))
        .filter(event_stickies::expires_at.gt(now.0 as i64))
        .filter(event_stickies::deliver_sn.lt(until_sn))
        .into_boxed();

    if let Some(since_sn) = since_sn {
        query = query.filter(event_stickies::deliver_sn.ge(since_sn));
    }

    let rows = query
        .order(event_stickies::deliver_sn.asc())
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
        let mut sticky = pdu(1_000_000, Some(json!({ "duration_ms": 300_000 })));
        sticky.state_key = Some(String::new());

        // Every client-facing shape, not just the timeline one: a client that meets the
        // event through state or a bundled relation still has to see how long it sticks.
        for serialized in [
            serde_json::to_value(sticky.to_sync_room_event()).unwrap(),
            serde_json::to_value(sticky.to_room_event()).unwrap(),
            serde_json::to_value(sticky.to_message_like_event()).unwrap(),
            serde_json::to_value(sticky.to_sync_state_event()).unwrap(),
            sticky.to_state_event_value(),
            serde_json::to_value(sticky.to_member_event()).unwrap(),
        ] {
            assert_eq!(serialized[STICKY_KEY], json!({ "duration_ms": 300_000 }));
        }

        // An out-of-range annotation is not a sticky event, so it is not echoed back to
        // clients as one -- including through the state shape, which otherwise copies
        // unknown top-level keys verbatim.
        let mut invalid = pdu(1_000_000, Some(json!({ "duration_ms": 3_600_001 })));
        invalid.state_key = Some(String::new());
        assert_eq!(
            serde_json::to_value(invalid.to_sync_room_event())
                .unwrap()
                .get(STICKY_KEY),
            None
        );
        assert_eq!(invalid.to_state_event_value().get(STICKY_KEY), None);

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
