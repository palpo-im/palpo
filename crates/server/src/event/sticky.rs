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

async fn lock_sticky_stream_shared(conn: &mut AsyncPgConnection) -> Result<(), DieselError> {
    diesel::sql_query("SELECT pg_advisory_xact_lock_shared($1)")
        .bind::<diesel::sql_types::BigInt, _>(STICKY_STREAM_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

/// Read the global stream position after all earlier presence, sticky and device-inbox
/// writes that can affect this `/sync` have committed.
///
/// All three allocate from `occur_sn_seq`. Taking their advisory locks in one
/// transaction is essential: two separate snapshots could let the second one observe a
/// sequence number allocated by an uncommitted writer that started after the first lock
/// was released, causing the returned sync token to skip that event permanently.
pub async fn curr_sn_after_sync_writes(
    user_id: &UserId,
    device_id: &DeviceId,
) -> AppResult<Seqnum> {
    let curr_sn =
        connect()
            .await?
            .transaction::<_, crate::AppError, _>(async |conn| {
                // Same order as `curr_sn_after_presence_writes`: presence, then inbox. Sync
                // readers may snapshot concurrently; they only need to exclude the exclusive
                // writers that allocate a stream position.
                crate::data::user::lock_presence_stream_shared(conn).await?;
                lock_sticky_stream_shared(conn).await?;
                crate::data::user::device::lock_inbox_stream(conn, user_id, device_id).await?;
                Ok(diesel::dsl::sql::<diesel::sql_types::BigInt>(
                    "SELECT last_value FROM occur_sn_seq",
                )
                .get_result::<Seqnum>(conn)
                .await?)
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
pub async fn record_with_conn(
    conn: &mut AsyncPgConnection,
    pdu: &PduEvent,
    event_sn: Seqnum,
    received_at: UnixMillis,
) -> AppResult<()> {
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
        .execute(conn)
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
pub async fn promote_to_timeline_with_conn(
    conn: &mut AsyncPgConnection,
    pdu: &SnPduEvent,
) -> AppResult<()> {
    // Ordinary events need no sticky-table query, advisory lock, or extra sequence
    // allocation inside the caller's event-persistence transaction.
    if pdu.sticky_duration_ms().is_none() {
        diesel::update(events::table.find(&*pdu.event_id))
            .set((
                events::is_outlier.eq(false),
                events::soft_failed.eq(pdu.soft_failed),
            ))
            .execute(conn)
            .await?;
        return Ok(());
    }

    {
        lock_sticky_stream(conn).await?;

        // `received_at` is persisted with newly stored events. The fallback covers
        // events created by older code and direct membership paths which did not
        // persist it; those reach promotion immediately, so `now` is their receipt
        // time for practical purposes.
        // Lock the event row so a concurrent redaction and promotion have a defined
        // order. Without this, promotion could read the old unredacted PDU, race with
        // `redact_pdu` deleting its sticky row, and recreate that row after the
        // redaction committed.
        let (stored_received_at, is_redacted, is_rejected) = events::table
            .find(&*pdu.event_id)
            .select((
                events::received_at,
                events::is_redacted,
                events::is_rejected,
            ))
            .for_update()
            .first::<(Option<i64>, bool, bool)>(conn)
            .await?;
        // A rejected event -- including one the room's Policy Server refused (MSC4284),
        // which is persisted as a rejection -- and a soft-failed one are never delivered
        // as sticky. The stored verdict is checked as well as the PDU's, since a replay
        // may carry none.
        let deliverable =
            !is_redacted && !is_rejected && pdu.rejection_reason.is_none() && !pdu.soft_failed;
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
        } else if deliverable
            && let Some(expires_at) = pdu.sticky_expires_at(received_at)
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
        if deliverable {
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

        // `process_to_timeline_pdu` clears the PDU's soft-failed flag once the event is
        // re-authorised (and policy-checked), including on sticky re-evaluation.
        diesel::update(events::table.find(&*pdu.event_id))
            .set((
                events::is_outlier.eq(false),
                events::soft_failed.eq(pdu.soft_failed),
            ))
            .execute(conn)
            .await?;
    }
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

/// Remaining TTLs for sticky events already present in a normal timeline response.
///
/// This lookup deliberately ignores `deliver_sn`: an event can be visible through the
/// normal timeline while it is still an outlier and has not been promoted onto the sticky
/// delivery stream. The timeline copy still needs a TTL, and is already subject to the
/// timeline's ordinary visibility checks.
pub async fn timeline_ttls(
    event_sns: &[Seqnum],
    now: UnixMillis,
) -> AppResult<std::collections::BTreeMap<Seqnum, u64>> {
    if event_sns.is_empty() {
        return Ok(Default::default());
    }

    let rows = event_stickies::table
        .filter(event_stickies::event_sn.eq_any(event_sns))
        .filter(event_stickies::expires_at.gt(now.0 as i64))
        .select((event_stickies::event_sn, event_stickies::expires_at))
        .load::<(Seqnum, i64)>(&mut connect().await?)
        .await?;

    let mut ttls = event_sns
        .iter()
        .map(|sn| (*sn, 0))
        .collect::<std::collections::BTreeMap<_, _>>();
    for (event_sn, expires_at) in rows {
        ttls.insert(event_sn, ttl_ms(UnixMillis(expires_at as u64), now));
    }
    Ok(ttls)
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

/// Whether the event is currently sticky and has reached the timeline.
///
/// Soft-failed, rejected and outlier events never get a delivery position, so they gain
/// none of the sticky privileges below.
async fn is_delivered_and_sticky(event_id: &EventId, now: UnixMillis) -> AppResult<bool> {
    let query = event_stickies::table
        .filter(event_stickies::event_id.eq(event_id))
        .filter(event_stickies::deliver_sn.is_not_null())
        .filter(event_stickies::expires_at.gt(now.0 as i64));
    Ok(crate::data::diesel_exists!(query, &mut connect().await?)?)
}

/// MSC4354: history visibility is not applied to sticky events. Any currently joined
/// user may see one for as long as it stays sticky.
///
/// Only consults the database for events that carry a valid sticky object, so ordinary
/// events cost nothing extra.
pub async fn user_can_see_while_sticky(pdu: &PduEvent, user_id: &UserId) -> AppResult<bool> {
    if pdu.sticky_duration_ms().is_none()
        || !is_delivered_and_sticky(&pdu.event_id, UnixMillis::now()).await?
    {
        return Ok(false);
    }
    crate::room::user::is_joined(user_id, &pdu.room_id).await
}

/// Server counterpart of [`user_can_see_while_sticky`], for federation `/event`,
/// `/backfill` and the other endpoints guarded by `server_can_see_event`.
pub async fn server_can_see_while_sticky(pdu: &PduEvent, server: &ServerName) -> AppResult<bool> {
    if pdu.sticky_duration_ms().is_none()
        || !is_delivered_and_sticky(&pdu.event_id, UnixMillis::now()).await?
    {
        return Ok(false);
    }
    crate::room::is_server_joined(server, &pdu.room_id).await
}

/// This server's own unexpired sticky events in the room, in creation order.
///
/// Only events that reached the timeline qualify; redacted events have already lost their
/// sticky row.
///
/// The sender is read from the event JSON: `events.sender_id` is not populated for every
/// event.
pub async fn own_unexpired(
    room_id: &RoomId,
    server_name: &ServerName,
    now: UnixMillis,
) -> AppResult<Vec<OwnedEventId>> {
    let rows = event_stickies::table
        .inner_join(events::table.on(events::id.eq(event_stickies::event_id)))
        .inner_join(event_datas::table.on(event_datas::event_id.eq(event_stickies::event_id)))
        .filter(event_stickies::room_id.eq(room_id))
        .filter(event_stickies::expires_at.gt(now.0 as i64))
        .filter(event_stickies::deliver_sn.is_not_null())
        .filter(events::is_outlier.eq(false))
        .filter(events::is_rejected.eq(false))
        .filter(events::is_redacted.eq(false))
        .order(event_stickies::event_sn.asc())
        .select((
            event_stickies::event_id,
            diesel::dsl::sql::<diesel::sql_types::Nullable<diesel::sql_types::Text>>(
                "event_datas.json_data ->> 'sender'",
            ),
        ))
        .load::<(OwnedEventId, Option<String>)>(&mut connect().await?)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, sender)| {
            sender
                .as_deref()
                .and_then(|sender| UserId::parse(sender).ok())
                .is_some_and(|sender| sender.server_name() == server_name)
        })
        .map(|(event_id, _)| event_id)
        .collect())
}

/// MSC4354: when a server newly joins the room, push it all of our own unexpired sticky
/// events.
///
/// They go through the normal durable federation queue, which honours per-server backoff
/// and never drops queued PDUs under load. Enqueueing in creation order gives the
/// best-effort ordering the MSC asks for.
pub async fn push_own_to_new_server(room_id: &RoomId, server: &ServerName) -> AppResult<()> {
    let own = own_unexpired(room_id, crate::config::server_name(), UnixMillis::now()).await?;
    for event_id in own {
        crate::sending::send_pdu_servers(std::iter::once(server.to_owned()), &event_id).await?;
    }
    Ok(())
}

/// Unexpired sticky events in the room that are held back as soft failed.
pub async fn soft_failed_candidates(
    room_id: &RoomId,
    now: UnixMillis,
) -> AppResult<Vec<OwnedEventId>> {
    event_stickies::table
        .inner_join(events::table.on(events::id.eq(event_stickies::event_id)))
        .filter(event_stickies::room_id.eq(room_id))
        .filter(event_stickies::expires_at.gt(now.0 as i64))
        .filter(event_stickies::deliver_sn.is_null())
        .filter(events::is_outlier.eq(true))
        .filter(events::soft_failed.eq(true))
        .filter(events::is_rejected.eq(false))
        .filter(events::is_redacted.eq(false))
        .order(event_stickies::event_sn.asc())
        .select(event_stickies::event_id)
        .load::<OwnedEventId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// MSC4354: re-evaluates the soft-failure of the room's unexpired sticky events after its
/// current state changed, promoting those that now pass to the timeline (and so to
/// `/sync`).
///
/// Runs detached: the caller holds the room's state lock, which the promotion path takes
/// itself. Expired events are no longer candidates, so re-evaluation stops with the
/// stickiness.
pub fn reevaluate_soft_failed_later(room_id: OwnedRoomId) {
    tokio::spawn(async move {
        if let Err(e) = reevaluate_soft_failed(&room_id).await {
            tracing::warn!(%room_id, error = ?e, "failed to re-evaluate soft-failed sticky events");
        }
    });
}

/// Re-runs the soft-fail check for the room's soft-failed sticky events and processes those
/// that now pass through the normal timeline path. Returns how many reached the timeline.
pub async fn reevaluate_soft_failed(room_id: &RoomId) -> AppResult<usize> {
    let candidates = soft_failed_candidates(room_id, UnixMillis::now()).await?;
    if candidates.is_empty() {
        return Ok(0);
    }
    let room_version = crate::room::get_version(room_id).await?;
    let mut promoted = 0;
    for event_id in candidates {
        let pdu = match crate::room::timeline::get_pdu(&event_id).await {
            Ok(pdu) => pdu,
            Err(e) if e.is_not_found() => continue,
            Err(e) => return Err(e),
        };
        // Checked first so an event that still fails leaves no trace: the full
        // timeline path would record it as soft failed again.
        if crate::event::handler::fails_current_state_check(&pdu, &room_version).await? {
            continue;
        }
        let Some(json) = crate::room::timeline::get_pdu_json(&event_id).await? else {
            continue;
        };
        match crate::event::handler::process_to_timeline_pdu(pdu, json, None).await {
            Ok(()) => promoted += 1,
            Err(e) => {
                tracing::debug!(%event_id, error = ?e, "soft-failed sticky event still not accepted");
            }
        }
    }
    Ok(promoted)
}

/// The sync position sticky events are delivered from ([MSC4354]).
///
/// `None` means every unexpired sticky event in the room: on an initial sync, and in the
/// sync following the user's join, the MSC requires all of them regardless of `since`.
///
/// [MSC4354]: https://github.com/matrix-org/matrix-spec-proposals/pull/4354
pub fn delivery_since(since_sn: Option<Seqnum>, joined_since: bool) -> Option<Seqnum> {
    if joined_since { None } else { since_sn }
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
            transaction_device: None,
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
        let recipient = UserId::parse("@bob:example.org").unwrap();
        let recipient = &*recipient;
        let mut sticky = pdu(1_000_000, Some(json!({ "duration_ms": 300_000 })));
        sticky.state_key = Some(String::new());

        // Every client-facing shape, not just the timeline one: a client that meets the
        // event through state or a bundled relation still has to see how long it sticks.
        for serialized in [
            serde_json::to_value(sticky.to_sync_room_event_for(recipient, None)).unwrap(),
            serde_json::to_value(sticky.to_room_event_for(recipient, None)).unwrap(),
            serde_json::to_value(sticky.to_message_like_event_for(recipient, None)).unwrap(),
            serde_json::to_value(sticky.to_sync_state_event_for(recipient, None)).unwrap(),
            sticky.to_state_event_value_for(recipient, None),
            serde_json::to_value(sticky.to_member_event_for(recipient, None)).unwrap(),
        ] {
            assert_eq!(serialized[STICKY_KEY], json!({ "duration_ms": 300_000 }));
        }

        // An out-of-range annotation is not a sticky event, so it is not echoed back to
        // clients as one -- including through the state shape, which otherwise copies
        // unknown top-level keys verbatim.
        let mut invalid = pdu(1_000_000, Some(json!({ "duration_ms": 3_600_001 })));
        invalid.state_key = Some(String::new());
        assert_eq!(
            serde_json::to_value(invalid.to_sync_room_event_for(recipient, None))
                .unwrap()
                .get(STICKY_KEY),
            None
        );
        assert_eq!(
            invalid
                .to_state_event_value_for(recipient, None)
                .get(STICKY_KEY),
            None
        );

        // An ordinary event is untouched.
        let ordinary = pdu(1_000_000, None);
        assert_eq!(
            serde_json::to_value(ordinary.to_sync_room_event_for(recipient, None))
                .unwrap()
                .get(STICKY_KEY),
            None
        );
    }

    #[test]
    fn event_json_cannot_make_a_builder_sticky() {
        // `/createRoom` deserializes `initial_state` entries straight into `PduBuilder`;
        // stickiness is requested only through the validated send query parameter.
        let builder: crate::event::PduBuilder = serde_json::from_value(json!({
            "type": "m.room.topic",
            "content": {},
            "sticky_duration_ms": 60_000,
        }))
        .unwrap();
        assert!(builder.sticky_duration_ms.is_none());
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
    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_sticky_save_rolls_back_and_expiry_stays_expired() {
        use super::*;
        use crate::event::OutlierPdu;
        crate::test_database::init();
        let now = UnixMillis::now();
        let event = pdu(now.0, Some(json!({"duration_ms": 60_000})));
        let outlier = OutlierPdu {
            pdu: event.clone(),
            json_data: crate::core::serde::to_canonical_object(&event).unwrap(),
            soft_failed: false,
            policy_refused: false,
            remote_server: "example.org".try_into().unwrap(),
            room_id: event.room_id.clone(),
            room_version: crate::core::RoomVersionId::V11,
            event_sn: None,
        };
        let mut conn = connect().await.unwrap();
        // Scoped to this event so concurrently running database tests are unaffected.
        diesel::sql_query("CREATE FUNCTION reject_sticky_test() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id = '$event:example.org' THEN RAISE EXCEPTION 'injected sticky failure'; END IF; RETURN NEW; END $$")
            .execute(&mut conn).await.unwrap();
        diesel::sql_query("CREATE TRIGGER reject_sticky_test BEFORE INSERT ON event_stickies FOR EACH ROW EXECUTE FUNCTION reject_sticky_test()")
            .execute(&mut conn).await.unwrap();
        assert!(outlier.clone().save_to_database(false).await.is_err());
        assert_eq!(
            events::table
                .find(&event.event_id)
                .count()
                .get_result::<i64>(&mut conn)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            event_datas::table
                .find(&event.event_id)
                .count()
                .get_result::<i64>(&mut conn)
                .await
                .unwrap(),
            0
        );
        diesel::sql_query("DROP TRIGGER reject_sticky_test ON event_stickies")
            .execute(&mut conn)
            .await
            .unwrap();
        let (stored, _, _guard) = outlier.save_to_database(false).await.unwrap();
        let failed = conn
            .transaction::<(), crate::AppError, _>(async |conn| {
                promote_to_timeline_with_conn(conn, &stored).await?;
                Err(crate::AppError::internal("simulate promotion rollback"))
            })
            .await;
        assert!(failed.is_err());
        assert!(
            events::table
                .find(&event.event_id)
                .select(events::is_outlier)
                .first::<bool>(&mut conn)
                .await
                .unwrap()
        );
        assert!(
            event_stickies::table
                .find(&event.event_id)
                .select(event_stickies::deliver_sn)
                .first::<Option<i64>>(&mut conn)
                .await
                .unwrap()
                .is_none()
        );
        conn.transaction::<(), crate::AppError, _>(async |conn| {
            promote_to_timeline_with_conn(conn, &stored).await
        })
        .await
        .unwrap();
        assert!(
            !unexpired(&event.room_id, None, i64::MAX, now)
                .await
                .unwrap()
                .is_empty()
        );
        let expired = UnixMillis(now.0 + 61_000);
        delete_expired(expired).await.unwrap();
        assert_eq!(
            timeline_ttls(&[stored.event_sn], expired)
                .await
                .unwrap()
                .get(&stored.event_sn),
            Some(&0)
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_resaved_outlier_keeps_its_first_receipt_time() {
        use super::*;
        use crate::event::OutlierPdu;
        crate::test_database::init();
        let now = UnixMillis::now();
        // A sender clock far in the future: the window is measured from our receipt.
        let mut event = pdu(now.0 + 86_400_000, Some(json!({"duration_ms": 60_000})));
        event.event_id = EventId::parse("$resaved:example.org").unwrap().to_owned();
        let outlier = OutlierPdu {
            pdu: event.clone(),
            json_data: crate::core::serde::to_canonical_object(&event).unwrap(),
            soft_failed: false,
            policy_refused: false,
            remote_server: "example.org".try_into().unwrap(),
            room_id: event.room_id.clone(),
            room_version: crate::core::RoomVersionId::V11,
            event_sn: None,
        };
        outlier.clone().save_to_database(false).await.unwrap();

        // The first receipt was long enough ago that the window has closed, and the reaper
        // has removed the sticky row.
        let first_receipt = now.0 as i64 - 120_000;
        let mut conn = connect().await.unwrap();
        diesel::update(events::table.find(&event.event_id))
            .set(events::received_at.eq(first_receipt))
            .execute(&mut conn)
            .await
            .unwrap();
        diesel::delete(event_stickies::table.find(&event.event_id))
            .execute(&mut conn)
            .await
            .unwrap();

        // A retransmission must not restart the window.
        let (stored, _, _guard) = outlier.save_to_database(false).await.unwrap();
        assert_eq!(
            events::table
                .find(&event.event_id)
                .select(events::received_at)
                .first::<Option<i64>>(&mut conn)
                .await
                .unwrap(),
            Some(first_receipt)
        );
        conn.transaction::<(), crate::AppError, _>(async |conn| {
            promote_to_timeline_with_conn(conn, &stored).await
        })
        .await
        .unwrap();
        assert_eq!(
            event_stickies::table
                .find(&event.event_id)
                .count()
                .get_result::<i64>(&mut conn)
                .await
                .unwrap(),
            0
        );
    }

    #[test]
    fn sync_after_join_delivers_every_unexpired_sticky_event() {
        use super::delivery_since;
        assert_eq!(delivery_since(Some(42), false), Some(42));
        // MSC4354: the sync following a join carries all unexpired sticky events, not
        // just those that became deliverable since the previous sync.
        assert_eq!(delivery_since(Some(42), true), None);
        assert_eq!(delivery_since(None, false), None);
    }

    #[tokio::test]
    async fn only_redactions_are_soft_failed_against_current_state() {
        let event = pdu(1_000, Some(json!({"duration_ms": 60_000})));
        assert!(
            !crate::event::handler::fails_current_state_check(
                &event,
                &crate::core::RoomVersionId::V11
            )
            .await
            .unwrap()
        );
    }

    /// Stores `event` as an outlier, as federation ingestion does.
    async fn store_outlier(event: &PduEvent, soft_failed: bool) -> crate::event::SnPduEvent {
        store_outlier_with(event, soft_failed, false).await
    }

    /// Stores `event` as an outlier, optionally refused by the room's Policy Server.
    async fn store_outlier_with(
        event: &PduEvent,
        soft_failed: bool,
        policy_refused: bool,
    ) -> crate::event::SnPduEvent {
        let outlier = crate::event::OutlierPdu {
            pdu: event.clone(),
            json_data: crate::core::serde::to_canonical_object(event).unwrap(),
            soft_failed,
            policy_refused,
            remote_server: "example.org".try_into().unwrap(),
            room_id: event.room_id.clone(),
            room_version: crate::core::RoomVersionId::V11,
            event_sn: None,
        };
        outlier.save_to_database(false).await.unwrap().0
    }

    async fn promote(stored: &crate::event::SnPduEvent) {
        use diesel_async::AsyncConnection;

        use crate::data::connect;
        connect()
            .await
            .unwrap()
            .transaction::<(), crate::AppError, _>(async |conn| {
                super::promote_to_timeline_with_conn(conn, stored).await
            })
            .await
            .unwrap();
    }

    fn sticky_event(event_id: &str, room_id: &str, sender: &str, now: UnixMillis) -> PduEvent {
        let mut event = pdu(now.0, Some(json!({"duration_ms": 60_000})));
        event.event_id = EventId::parse(event_id).unwrap().to_owned();
        event.room_id = RoomId::parse(room_id).unwrap().to_owned();
        event.sender = UserId::parse(sender).unwrap().to_owned();
        event
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_new_joiner_is_sent_only_our_unexpired_sticky_events_in_order() {
        use super::*;
        crate::test_database::init();
        let room = "!joiner:example.org";
        let now = UnixMillis::now();

        // A server only counts as newly joined the first time it is recorded.
        let room_id = RoomId::parse(room).unwrap();
        let room_id = &*room_id;
        let other: &ServerName = "other.org".try_into().unwrap();
        assert!(
            crate::data::room::add_joined_server(room_id, other)
                .await
                .unwrap()
        );
        assert!(
            !crate::data::room::add_joined_server(room_id, other)
                .await
                .unwrap()
        );

        let first = store_outlier(
            &sticky_event("$own1:example.org", room, "@alice:example.org", now),
            false,
        )
        .await;
        let remote = store_outlier(
            &sticky_event("$remote:other.org", room, "@bob:other.org", now),
            false,
        )
        .await;
        let second = store_outlier(
            &sticky_event("$own2:example.org", room, "@alice:example.org", now),
            false,
        )
        .await;
        // Never reached the timeline, so there is nothing to push.
        store_outlier(
            &sticky_event("$pending:example.org", room, "@alice:example.org", now),
            true,
        )
        .await;
        // Promoted out of creation order: the push still follows creation order.
        promote(&second).await;
        promote(&remote).await;
        promote(&first).await;

        let own: &ServerName = "example.org".try_into().unwrap();
        assert_eq!(
            own_unexpired(room_id, own, now).await.unwrap(),
            vec![first.event_id.clone(), second.event_id.clone()]
        );
        // Stickiness over, nothing left to push.
        assert!(
            own_unexpired(room_id, own, UnixMillis(now.0 + 61_000))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_soft_failed_sticky_event_is_reevaluated_until_promoted_or_expired() {
        use super::*;
        crate::test_database::init();
        let room = "!softfail:example.org";
        let room_id = RoomId::parse(room).unwrap();
        let room_id = &*room_id;
        let now = UnixMillis::now();
        let held = store_outlier(
            &sticky_event("$held:other.org", room, "@bob:other.org", now),
            true,
        )
        .await;
        store_outlier(
            &sticky_event("$ok:other.org", room, "@bob:other.org", now),
            false,
        )
        .await;

        assert_eq!(
            soft_failed_candidates(room_id, now).await.unwrap(),
            vec![held.event_id.clone()]
        );
        // Re-evaluation stops once the event is no longer sticky.
        assert!(
            soft_failed_candidates(room_id, UnixMillis(now.0 + 61_000))
                .await
                .unwrap()
                .is_empty()
        );

        // Passing the re-evaluation promotes it like any accepted event, which delivers
        // it to /sync and clears the soft-failed mark.
        promote(&held).await;
        let mut conn = connect().await.unwrap();
        assert!(
            !events::table
                .find(&held.event_id)
                .select(events::soft_failed)
                .first::<bool>(&mut conn)
                .await
                .unwrap()
        );
        assert!(
            soft_failed_candidates(room_id, now)
                .await
                .unwrap()
                .is_empty()
        );
        let delivered = unexpired(room_id, None, i64::MAX, now).await.unwrap();
        assert!(
            delivered
                .iter()
                .any(|entry| entry.event_id == held.event_id)
        );
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_joined_users_and_servers_see_sticky_events_regardless_of_visibility() {
        use super::*;
        use crate::data::room::NewDbRoomUser;
        crate::test_database::init();
        let room = "!visible:example.org";
        let room_id = RoomId::parse(room).unwrap();
        let room_id = &*room_id;
        let now = UnixMillis::now();
        let carol = UserId::parse("@carol:example.org").unwrap();
        let carol = &*carol;
        let dave = UserId::parse("@dave:example.org").unwrap();
        let dave = &*dave;
        let joined_server: &ServerName = "joined.org".try_into().unwrap();
        let other_server: &ServerName = "elsewhere.org".try_into().unwrap();

        let mut conn = connect().await.unwrap();
        diesel::insert_into(room_users::table)
            .values(NewDbRoomUser {
                event_id: EventId::parse("$carol-join:example.org")
                    .unwrap()
                    .to_owned(),
                event_sn: 1,
                room_id: room_id.to_owned(),
                room_server_id: None,
                user_id: carol.to_owned(),
                user_server_id: carol.server_name().to_owned(),
                sender_id: carol.to_owned(),
                membership: "join".to_owned(),
                forgotten: false,
                display_name: None,
                avatar_url: None,
                state_data: None,
                created_at: now,
            })
            .execute(&mut conn)
            .await
            .unwrap();
        crate::data::room::add_joined_server(room_id, joined_server)
            .await
            .unwrap();

        let stored = store_outlier(
            &sticky_event("$seen:other.org", room, "@bob:other.org", now),
            false,
        )
        .await;
        // Not delivered yet: no sticky privilege.
        assert!(!user_can_see_while_sticky(&stored, carol).await.unwrap());
        promote(&stored).await;

        assert!(user_can_see_while_sticky(&stored, carol).await.unwrap());
        // The bypass is reached from the ordinary visibility check, which has no
        // event-time state for this event at all.
        assert!(stored.user_can_see(carol).await.unwrap());
        assert!(!user_can_see_while_sticky(&stored, dave).await.unwrap());
        assert!(
            server_can_see_while_sticky(&stored, joined_server)
                .await
                .unwrap()
        );
        assert!(
            !server_can_see_while_sticky(&stored, other_server)
                .await
                .unwrap()
        );

        // Once the stickiness ends, history visibility applies again.
        diesel::update(event_stickies::table.find(&stored.event_id))
            .set(event_stickies::expires_at.eq(now.0 as i64 - 1))
            .execute(&mut conn)
            .await
            .unwrap();
        assert!(!user_can_see_while_sticky(&stored, carol).await.unwrap());
        assert!(
            !server_can_see_while_sticky(&stored, joined_server)
                .await
                .unwrap()
        );

        // Ordinary events never qualify.
        let mut plain = sticky_event("$plain:other.org", room, "@bob:other.org", now);
        plain.extra_data.clear();
        let plain = store_outlier(&plain, false).await;
        assert!(!user_can_see_while_sticky(&plain, carol).await.unwrap());
    }
}
