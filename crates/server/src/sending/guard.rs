use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use super::{
    EduBuf, EduVec, MPSC_SENDER, OutgoingKind, SELECT_EDU_LIMIT, SELECT_PRESENCE_LIMIT,
    SELECT_RECEIPT_LIMIT, SendingEventType, TransactionStatus,
};
use crate::core::device::DeviceListUpdateContent;
use crate::core::events::receipt::{ReceiptContent, ReceiptData, ReceiptMap, ReceiptType};
use crate::core::federation::transaction::Edu;
use crate::core::identifiers::*;
use crate::core::presence::{PresenceContent, PresenceUpdate};
use crate::core::{Seqnum, device_id};
use crate::exts::*;
use crate::room::state;
use crate::{AppResult, data, room};

/// How often the guard scans the database for queued requests whose wakeup
/// was dropped (full wakeup queue) or that were left over by a previous
/// process.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// A configured sending guard whose worker has not been started yet.
///
/// Keeping initialization separate from `start` lets startup admin commands
/// enqueue durable requests without dispatching network traffic in
/// `--server false` mode.
#[must_use = "call Guard::start when the server is enabled"]
pub struct Guard {
    receiver: mpsc::Receiver<super::WakeupMessage>,
}

pub fn init() -> Guard {
    let (sender, receiver) = mpsc::channel(super::WAKEUP_QUEUE_CAPACITY);
    assert!(
        MPSC_SENDER.set(sender).is_ok(),
        "sending guard is already initialized"
    );
    Guard { receiver }
}

impl Guard {
    pub fn start(self) {
        tokio::spawn(async move {
            if let Err(error) = process(self.receiver).await {
                error!(?error, "sending guard stopped");
            }
        });
    }
}

async fn process(mut receiver: mpsc::Receiver<super::WakeupMessage>) -> AppResult<()> {
    let mut futures = FuturesUnordered::new();
    let mut current_transaction_status = HashMap::<OutgoingKind, TransactionStatus>::new();
    // EDU selection windows of in-flight transactions; persisted as the
    // destination's cursor once the transaction succeeds.
    let mut pending_edu_cursors = HashMap::<OutgoingKind, Seqnum>::new();

    // Retry requests we could not finish yet
    let mut initial_transactions = HashMap::<OutgoingKind, Vec<SendingEventType>>::new();

    for (id, outgoing_kind, event) in super::active_requests().await? {
        let entry = initial_transactions
            .entry(outgoing_kind.clone())
            .or_default();

        if entry.len() > 512 {
            error!(
                "Too many pending events ({}) for {:?}, dropping oldest event {:?}",
                entry.len(),
                outgoing_kind,
                id
            );
            super::delete_request(id).await?;
            continue;
        }

        entry.push(event);
    }

    for (outgoing_kind, events) in initial_transactions {
        current_transaction_status.insert(outgoing_kind.clone(), TransactionStatus::Running);
        futures.push(super::send_events(outgoing_kind.clone(), events));
    }

    let mut sweep = tokio::time::interval(SWEEP_INTERVAL);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        let retry_delay = next_retry_delay(&current_transaction_status);

        tokio::select! {
            Some(response) = futures.next() => {
                match response {
                    Ok(outgoing_kind) => {
                        super::delete_all_active_requests_for(&outgoing_kind).await?;

                        // The transaction reached the destination; persist its EDU
                        // window so the next selection resumes after it.
                        if let Some(edu_sn) = pending_edu_cursors.remove(&outgoing_kind)
                            && let OutgoingKind::Normal(server_name) = &outgoing_kind
                            && let Err(e) = data::sending::advance_edu_cursor(server_name, edu_sn).await
                        {
                            error!(?server_name, error = ?e, "failed to advance edu cursor");
                        }

                        // Find events that have been added since starting the last request
                        let new_events = super::queued_requests(&outgoing_kind, super::QUEUED_REQUEST_LIMIT).await.unwrap_or_default();

                        let mut events = if new_events.is_empty() {
                            Vec::new()
                        } else {
                            // Claim only rows that are still queued. Another
                            // process may have claimed or completed them since
                            // the select above.
                            super::claim_queued_requests(&new_events).await?
                        };

                        if !events.is_empty() {
                            // Piggyback EDUs that accumulated while the previous
                            // transaction was in flight, so a busy destination
                            // does not starve presence/receipt/device-list updates.
                            if let OutgoingKind::Normal(server_name) = &outgoing_kind
                                && let Ok((select_edus, last_sn)) = select_edus(server_name).await
                            {
                                events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                                pending_edu_cursors.insert(outgoing_kind.clone(), last_sn);
                            }

                            futures.push(super::send_events(outgoing_kind.clone(), events));
                        } else if let OutgoingKind::Normal(server_name) = &outgoing_kind
                            && let Ok((select_edus, last_sn)) = select_edus(server_name).await
                            && !select_edus.is_empty()
                        {
                            // No queued PDUs, but the EDU window is not empty
                            // (e.g. a previous send failed and dropped it):
                            // deliver the EDUs on their own instead of leaving
                            // them until unrelated traffic shows up.
                            let events = select_edus.into_iter().map(SendingEventType::Edu).collect::<Vec<_>>();
                            pending_edu_cursors.insert(outgoing_kind.clone(), last_sn);
                            futures.push(super::send_events(outgoing_kind.clone(), events));
                        } else {
                            current_transaction_status.remove(&outgoing_kind);
                        }
                    }
                    Err((outgoing_kind, event)) => {
                        error!("failed to send event: {event:?}  outgoing_kind:{outgoing_kind:?}");
                        // Do not advance the cursor: the same EDU window is
                        // re-selected once the destination is reachable again.
                        pending_edu_cursors.remove(&outgoing_kind);
                        current_transaction_status.entry(outgoing_kind.clone()).and_modify(|e| *e = match e {
                            TransactionStatus::Running => {
                                TransactionStatus::Failed(1, Instant::now())
                            },
                            TransactionStatus::Retrying(n) => {
                                TransactionStatus::Failed(n.saturating_add(1), Instant::now())
                            },
                            TransactionStatus::Failed(_, _) => {
                                error!("Request that was not even running failed?!");
                                return
                            },
                        });
                        // Persist retry state to DB for cross-instance coordination
                        if let Some(TransactionStatus::Failed(tries, _)) = current_transaction_status.get(&outgoing_kind) {
                            let _ = persist_retry_state(&outgoing_kind, *tries).await;
                        }
                    }
                };
            },
            Some(outgoing_kind) = receiver.recv() => {
                if current_transaction_status.contains_key(&outgoing_kind) {
                    // A running or backing-off transaction will load queued
                    // rows when it settles or retries.
                    continue;
                }
                let new_events = match super::queued_requests(
                    &outgoing_kind,
                    super::QUEUED_REQUEST_LIMIT,
                ).await {
                    Ok(events) => events,
                    Err(e) => {
                        error!(?outgoing_kind, error = ?e, "failed to load queued requests for wakeup");
                        continue;
                    }
                };
                if new_events.is_empty() {
                    // The periodic sweep already handled this wakeup.
                    continue;
                }
                match select_events(
                    &outgoing_kind,
                    new_events,
                    &mut current_transaction_status,
                    &mut pending_edu_cursors,
                ).await {
                    Ok(Some(events)) => {
                        futures.push(super::send_events(outgoing_kind, events));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        error!(?outgoing_kind, error = ?e, "failed to select queued requests for wakeup");
                    }
                }
            }
            _ = sweep.tick() => {
                // Recover destinations whose wakeup was dropped because the
                // wakeup queue was full, and requests left queued by a
                // previous process.
                let kinds = match super::queued_kinds().await {
                    Ok(kinds) => kinds,
                    Err(e) => {
                        error!(error = ?e, "failed to load queued destinations for sweep");
                        continue;
                    }
                };
                for outgoing_kind in kinds {
                    if current_transaction_status.contains_key(&outgoing_kind) {
                        // Running, retrying, or backing off: the existing flow
                        // picks queued requests up when the transaction settles.
                        continue;
                    }
                    let new_events = match super::queued_requests(&outgoing_kind, super::QUEUED_REQUEST_LIMIT).await {
                        Ok(events) => events,
                        Err(e) => {
                            error!(?outgoing_kind, error = ?e, "failed to load queued requests for sweep");
                            continue;
                        }
                    };
                    if new_events.is_empty() {
                        continue;
                    }
                    match select_events(
                        &outgoing_kind,
                        new_events,
                        &mut current_transaction_status,
                        &mut pending_edu_cursors,
                    ).await {
                        Ok(Some(events)) => {
                            futures.push(super::send_events(outgoing_kind, events));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            error!(?outgoing_kind, error = ?e, "failed to select queued requests for sweep");
                        }
                    }
                }
            }
            _ = async {
                if let Some(delay) = retry_delay {
                    tokio::time::sleep(delay).await;
                } else {
                    pending::<()>().await;
                }
            } => {
                let retry_ready = current_transaction_status
                    .iter()
                    .filter_map(|(outgoing_kind, status)| match status {
                        TransactionStatus::Failed(tries, time)
                            if time.elapsed() >= retry_backoff(*tries) =>
                        {
                            Some((outgoing_kind.clone(), *tries))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();

                for (outgoing_kind, tries) in retry_ready {
                    let active_events = match super::active_requests_for(&outgoing_kind).await {
                        Ok(events) => events,
                        Err(e) => {
                            error!(
                                ?outgoing_kind,
                                error = ?e,
                                "failed to load active requests for retry"
                            );
                            current_transaction_status.insert(
                                outgoing_kind,
                                TransactionStatus::Failed(tries, Instant::now()),
                            );
                            continue;
                        }
                    };

                    let mut events = active_events.into_iter().map(|(_, event)| event).collect::<Vec<_>>();
                    // Also retry the EDU window whose delivery failed: it was
                    // dropped from pending_edu_cursors without advancing the
                    // cursor, so it is re-selected here instead of waiting for
                    // unrelated future traffic to this destination.
                    if let OutgoingKind::Normal(server_name) = &outgoing_kind
                        && let Ok((select_edus, last_sn)) = select_edus(server_name).await
                        && !select_edus.is_empty()
                    {
                        events.extend(select_edus.into_iter().map(SendingEventType::Edu));
                        pending_edu_cursors.insert(outgoing_kind.clone(), last_sn);
                    }

                    if events.is_empty() {
                        current_transaction_status.remove(&outgoing_kind);
                        continue;
                    }

                    current_transaction_status
                        .insert(outgoing_kind.clone(), TransactionStatus::Retrying(tries));
                    futures.push(super::send_events(outgoing_kind, events));
                }
            }
        }
    }
}

fn retry_backoff(tries: u32) -> Duration {
    let mut duration = Duration::from_secs(30) * tries * tries;
    if duration > Duration::from_secs(60 * 60 * 24) {
        duration = Duration::from_secs(60 * 60 * 24);
    }
    duration
}

fn next_retry_delay(
    current_transaction_status: &HashMap<OutgoingKind, TransactionStatus>,
) -> Option<Duration> {
    current_transaction_status
        .values()
        .filter_map(|status| match status {
            TransactionStatus::Failed(tries, time) => {
                Some(retry_backoff(*tries).saturating_sub(time.elapsed()))
            }
            _ => None,
        })
        .min()
}

#[tracing::instrument(skip_all)]
async fn select_events(
    outgoing_kind: &OutgoingKind,
    new_events: Vec<(i64, SendingEventType)>, // Events we want to send: event and full key
    current_transaction_status: &mut HashMap<OutgoingKind, TransactionStatus>,
    pending_edu_cursors: &mut HashMap<OutgoingKind, Seqnum>,
) -> AppResult<Option<Vec<SendingEventType>>> {
    let mut retry = false;
    let mut allow = true;

    let entry = current_transaction_status.entry(outgoing_kind.clone());

    entry
        .and_modify(|e| match e {
            TransactionStatus::Running | TransactionStatus::Retrying(_) => {
                allow = false; // already running
            }
            TransactionStatus::Failed(tries, time) => {
                // Fail if a request has failed recently (exponential backoff)
                let min_elapsed_duration = retry_backoff(*tries);

                if time.elapsed() < min_elapsed_duration {
                    allow = false;
                } else {
                    retry = true;
                    *e = TransactionStatus::Retrying(*tries);
                }
            }
        })
        .or_insert(TransactionStatus::Running);

    if !allow {
        return Ok(None);
    }

    let mut events = Vec::new();

    if retry {
        // We retry the previous transaction
        for (_, e) in super::active_requests_for(outgoing_kind).await? {
            events.push(e);
        }
    } else {
        events = match super::claim_queued_requests(&new_events).await {
            Ok(events) => events,
            Err(error) => {
                current_transaction_status.remove(outgoing_kind);
                return Err(error);
            }
        };
        if events.is_empty() {
            // The wakeup was stale: its row was already claimed or deleted by
            // the periodic sweep (or another process). Undo the Running state
            // inserted above and do not start an empty transaction.
            current_transaction_status.remove(outgoing_kind);
            return Ok(None);
        }
    }

    // Piggyback pending EDUs on fresh and retry transactions alike. The
    // cursor only advances once a transaction succeeds, so a retry simply
    // re-selects the same window.
    if let OutgoingKind::Normal(server_name) = outgoing_kind
        && let Ok((select_edus, last_sn)) = select_edus(server_name).await
    {
        events.extend(select_edus.into_iter().map(SendingEventType::Edu));
        pending_edu_cursors.insert(outgoing_kind.clone(), last_sn);
    }

    Ok(Some(events))
}

/// Persist retry state to the database so other instances can respect backoff.
async fn persist_retry_state(outgoing_kind: &OutgoingKind, tries: u32) -> AppResult<()> {
    let dest = match outgoing_kind {
        OutgoingKind::Normal(server_name) => {
            data::sending::OutgoingDestination::Normal(server_name)
        }
        OutgoingKind::Appservice(id) => data::sending::OutgoingDestination::Appservice(id),
        OutgoingKind::Push(user_id, pushkey) => {
            data::sending::OutgoingDestination::Push { user_id, pushkey }
        }
    };
    data::sending::persist_retry_state(dest, tries).await?;
    Ok(())
}

/// Look for device changes
#[tracing::instrument(level = "trace", skip(server_name))]
async fn select_edus_device_changes(
    server_name: &ServerName,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    events_len: &AtomicUsize,
) -> AppResult<EduVec> {
    let mut events = EduVec::new();
    let server_rooms = state::server_joined_rooms(server_name).await?;

    let mut device_list_changes = HashSet::<OwnedUserId>::new();
    for room_id in server_rooms {
        let keys_changed = room::keys_changed_users(&room_id, since_sn, None)
            .await?
            .into_iter()
            .filter(|user_id| user_id.is_local());

        for user_id in keys_changed {
            // max_edu_sn.fetch_max(event_sn, Ordering::Relaxed);
            if !device_list_changes.insert(user_id.clone()) {
                continue;
            }

            // Empty prev id forces synapse to resync; because synapse resyncs,
            // we can just insert placeholder data
            let edu = Edu::DeviceListUpdate(DeviceListUpdateContent {
                user_id,
                device_id: device_id!("placeholder").to_owned(),
                device_display_name: Some("Placeholder".to_owned()),
                stream_id: 1,
                prev_id: Vec::new(),
                deleted: None,
                keys: None,
            });

            let mut buf = EduBuf::new();
            if let Err(e) = serde_json::to_writer(&mut buf, &edu) {
                tracing::error!("failed to serialize device list update to JSON: {e}");
                continue;
            }

            events.push(buf);
            if events_len.fetch_add(1, Ordering::Relaxed) >= SELECT_EDU_LIMIT - 1 {
                return Ok(events);
            }
        }
    }

    Ok(events)
}

/// Look for read receipts in this room
#[tracing::instrument(level = "trace", skip(server_name, max_edu_sn))]
async fn select_edus_receipts(
    server_name: &ServerName,
    since_sn: Seqnum,
    max_edu_sn: &Seqnum,
) -> AppResult<Option<EduBuf>> {
    let mut num = 0;
    let mut receipts: BTreeMap<OwnedRoomId, ReceiptMap> = BTreeMap::new();
    for room_id in state::server_joined_rooms(server_name).await? {
        let Ok(receipt_map) =
            select_edus_receipts_room(&room_id, since_sn, max_edu_sn, &mut num).await
        else {
            continue;
        };

        if !receipt_map.read.is_empty() {
            receipts.insert(room_id, receipt_map);
        }
    }

    if receipts.is_empty() {
        return Ok(None);
    }

    let receipt_content = Edu::Receipt(ReceiptContent::new(receipts));

    let mut buf = EduBuf::new();
    if let Err(e) = serde_json::to_writer(&mut buf, &receipt_content) {
        tracing::error!("failed to serialize Receipt EDU to JSON: {e}");
        return Ok(None);
    }

    Ok(Some(buf))
}
/// Look for read receipts in this room
#[tracing::instrument(level = "trace", skip(since_sn))]
async fn select_edus_receipts_room(
    room_id: &RoomId,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    num: &mut usize,
) -> AppResult<ReceiptMap> {
    let receipts = data::room::receipt::read_receipts(room_id, since_sn).await?;

    let mut read = BTreeMap::<OwnedUserId, ReceiptData>::new();
    for (user_id, read_receipt) in receipts {
        // if count > since_sn {
        //     break;
        // }

        // max_edu_sn.fetch_max(occur_sn, Ordering::Relaxed);
        if !user_id.is_local() {
            continue;
        }

        // let Ok(event) = serde_json::from_str(read_receipt.inner().get()) else {
        //     error!(
        //         ?user_id,
        //         ?read_receipt,
        //         "Invalid edu event in read_receipts."
        //     );
        //     continue;
        // };

        // let AnySyncEphemeralRoomEvent::Receipt(r) = event else {
        //     error!(?user_id, ?event, "Invalid event type in read_receipts");
        //     continue;
        // };

        let (event_id, mut receipt) = read_receipt
            .0
            .into_iter()
            .next()
            .expect("we only use one event per read receipt");

        let receipt = receipt
            .remove(&ReceiptType::Read)
            .expect("our read receipts always set this")
            .remove(&user_id)
            .expect("our read receipts always have the user here");

        let receipt_data = ReceiptData {
            data: receipt,
            event_ids: vec![event_id.clone()],
        };

        if read.insert(user_id.to_owned(), receipt_data).is_none() {
            *num = num.saturating_add(1);
            if *num >= SELECT_RECEIPT_LIMIT {
                break;
            }
        }
    }

    Ok(ReceiptMap { read })
}

/// Look for presence
#[tracing::instrument(level = "trace", skip(server_name))]
async fn select_edus_presence(
    server_name: &ServerName,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
) -> AppResult<Option<EduBuf>> {
    let presences_since = crate::data::user::presences_since(since_sn).await?;

    let mut presence_updates = HashMap::<OwnedUserId, PresenceUpdate>::new();
    for (user_id, presence_event) in presences_since {
        // max_edu_sn.fetch_max(occur_sn, Ordering::Relaxed);
        if !user_id.is_local() {
            continue;
        }

        if !state::server_can_see_user(server_name, &user_id).await? {
            continue;
        }

        let update = PresenceUpdate {
            user_id: user_id.clone(),
            presence: presence_event.content.presence,
            currently_active: presence_event.content.currently_active.unwrap_or(false),
            status_msg: presence_event.content.status_msg,
            last_active_ago: presence_event.content.last_active_ago.unwrap_or(0),
        };

        presence_updates.insert(user_id, update);
        if presence_updates.len() >= SELECT_PRESENCE_LIMIT {
            break;
        }
    }

    if presence_updates.is_empty() {
        return Ok(None);
    }

    let presence_content = Edu::Presence(PresenceContent {
        push: presence_updates.into_values().collect(),
    });

    let mut buf = EduBuf::new();
    if let Err(e) = serde_json::to_writer(&mut buf, &presence_content) {
        tracing::error!("failed to serialize Presence EDU to JSON: {e}");
        return Ok(None);
    }

    Ok(Some(buf))
}

/// The lower bound (inclusive) of the next EDU selection window.
///
/// A destination with a persisted cursor resumes right after it; a
/// destination we have never completed an EDU transaction for starts at the
/// current sequence number so it is not flooded with historical updates.
fn edu_since_sn(cursor: Option<Seqnum>, max_edu_sn: Seqnum) -> Seqnum {
    cursor.map_or(max_edu_sn, |c| c + 1)
}

#[tracing::instrument(skip(server_name))]
pub async fn select_edus(server_name: &ServerName) -> AppResult<(EduVec, i64)> {
    let max_edu_sn = data::curr_sn().await?;
    let conf = crate::config::get();

    let cursor = data::sending::get_edu_cursor(server_name).await?;
    let since_sn = edu_since_sn(cursor, max_edu_sn);

    let events_len = AtomicUsize::default();
    let device_changes =
        select_edus_device_changes(server_name, since_sn, &max_edu_sn, &events_len).await?;

    let mut events = device_changes;
    if conf.read_receipt.allow_outgoing
        && let Some(receipts) = select_edus_receipts(server_name, since_sn, &max_edu_sn).await?
    {
        events.push(receipts);
    }

    if conf.presence.allow_outgoing
        && let Some(presence) = select_edus_presence(server_name, since_sn, &max_edu_sn).await?
    {
        events.push(presence);
    }

    Ok((events, max_edu_sn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edu_selection_resumes_after_cursor_and_starts_fresh_without_one() {
        // Selection queries are `occur_sn >= since_sn`, so resuming right
        // after the cursor neither re-sends nor skips updates.
        assert_eq!(edu_since_sn(Some(41), 100), 42);
        // No cursor yet: start at the current sequence number instead of
        // replaying history to a destination we have never sent EDUs to.
        assert_eq!(edu_since_sn(None, 100), 100);
    }

    #[test]
    fn retry_backoff_grows_quadratically_and_caps_at_one_day() {
        assert_eq!(retry_backoff(1), Duration::from_secs(30));
        assert_eq!(retry_backoff(2), Duration::from_secs(120));
        assert_eq!(retry_backoff(100), Duration::from_secs(60 * 60 * 24));
    }

    #[test]
    fn next_retry_delay_returns_zero_for_due_failed_transaction() {
        let mut statuses = HashMap::new();
        statuses.insert(
            OutgoingKind::Normal(OwnedServerName::try_from("remote.example").unwrap()),
            TransactionStatus::Failed(1, Instant::now() - Duration::from_secs(31)),
        );

        assert_eq!(next_retry_delay(&statuses), Some(Duration::ZERO));
    }

    #[test]
    fn next_retry_delay_ignores_non_failed_transactions() {
        let mut statuses = HashMap::new();
        statuses.insert(
            OutgoingKind::Normal(OwnedServerName::try_from("remote.example").unwrap()),
            TransactionStatus::Running,
        );

        assert_eq!(next_retry_delay(&statuses), None);
    }
}
