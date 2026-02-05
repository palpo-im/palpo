use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::{Mutex, mpsc};

use super::{
    EduBuf, EduVec, MPSC_RECEIVER, MPSC_SENDER, OutgoingKind, SELECT_EDU_LIMIT,
    SELECT_PRESENCE_LIMIT, SELECT_RECEIPT_LIMIT, SendingEventType, TransactionStatus,
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

pub fn start() {
    let (sender, receiver) = mpsc::unbounded_channel();
    let _ = MPSC_SENDER.set(sender);
    let _ = MPSC_RECEIVER.set(Mutex::new(receiver));
    tokio::spawn(async move {
        process().await.unwrap();
    });
}

async fn process() -> AppResult<()> {
    let mut receiver = MPSC_RECEIVER
        .get()
        .expect("receiver should exist")
        .lock()
        .await;
    let mut futures = FuturesUnordered::new();
    let mut current_transaction_status = HashMap::<OutgoingKind, TransactionStatus>::new();

    // Retry requests we could not finish yet
    let mut initial_transactions = HashMap::<OutgoingKind, Vec<SendingEventType>>::new();

    for (id, outgoing_kind, event) in super::active_requests()? {
        let entry = initial_transactions
            .entry(outgoing_kind.clone())
            .or_default();

        if entry.len() > 30 {
            warn!(
                "Dropping some current events: {:?} {:?} {:?}",
                id, outgoing_kind, event
            );
            super::delete_request(id)?;
            continue;
        }

        entry.push(event);
    }

    for (outgoing_kind, events) in initial_transactions {
        current_transaction_status.insert(outgoing_kind.clone(), TransactionStatus::Running);
        futures.push(super::send_events(outgoing_kind.clone(), events));
    }

    loop {
        tokio::select! {
            Some(response) = futures.next() => {
                match response {
                    Ok(outgoing_kind) => {
                        super::delete_all_active_requests_for(&outgoing_kind)?;

                        // Find events that have been added since starting the last request
                        let new_events = super::queued_requests(&outgoing_kind).unwrap_or_default().into_iter().take(30).collect::<Vec<_>>();

                        if !new_events.is_empty() {
                            // Insert pdus we found
                            super::mark_as_active(&new_events)?;

                            futures.push(
                                super::send_events(
                                    outgoing_kind.clone(),
                                    new_events.into_iter().map(|(_, event)| event).collect(),
                                )
                            );
                        } else {
                            current_transaction_status.remove(&outgoing_kind);
                        }
                    }
                    Err((outgoing_kind, event)) => {
                        error!("failed to send event: {event:?}  outgoing_kind:{outgoing_kind:?}");
                        current_transaction_status.entry(outgoing_kind).and_modify(|e| *e = match e {
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
                    }
                };
            },
            Some((outgoing_kind, event, id)) = receiver.recv() => {
                if let Ok(Some(events)) = select_events(
                    &outgoing_kind,
                    vec![(id, event)],
                    &mut current_transaction_status,
                ) {
                    futures.push(super::send_events(outgoing_kind, events));
                }
            }
        }
    }
}

#[tracing::instrument(skip_all)]
fn select_events(
    outgoing_kind: &OutgoingKind,
    new_events: Vec<(i64, SendingEventType)>, // Events we want to send: event and full key
    current_transaction_status: &mut HashMap<OutgoingKind, TransactionStatus>,
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
                let mut min_elapsed_duration = Duration::from_secs(30) * (*tries) * (*tries);
                if min_elapsed_duration > Duration::from_secs(60 * 60 * 24) {
                    min_elapsed_duration = Duration::from_secs(60 * 60 * 24);
                }

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
        for (_, e) in super::active_requests_for(outgoing_kind)? {
            events.push(e);
        }
    } else {
        super::mark_as_active(&new_events)?;
        for (_, e) in new_events {
            events.push(e);
        }

        if let OutgoingKind::Normal(server_name) = outgoing_kind
            && let Ok((select_edus, _last_count)) = select_edus(server_name)
        {
            events.extend(select_edus.into_iter().map(SendingEventType::Edu));
        }
    }

    Ok(Some(events))
}

/// Look for device changes
#[tracing::instrument(level = "trace", skip(server_name))]
fn select_edus_device_changes(
    server_name: &ServerName,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    events_len: &AtomicUsize,
) -> AppResult<EduVec> {
    let mut events = EduVec::new();
    let server_rooms = state::server_joined_rooms(server_name)?;

    let mut device_list_changes = HashSet::<OwnedUserId>::new();
    for room_id in server_rooms {
        let keys_changed = room::keys_changed_users(&room_id, since_sn, None)?
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
            serde_json::to_writer(&mut buf, &edu)
                .expect("failed to serialize device list update to JSON");

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
fn select_edus_receipts(
    server_name: &ServerName,
    since_sn: Seqnum,
    max_edu_sn: &Seqnum,
) -> AppResult<Option<EduBuf>> {
    let mut num = 0;
    let receipts: BTreeMap<OwnedRoomId, ReceiptMap> = state::server_joined_rooms(server_name)?
        .into_iter()
        .filter_map(|room_id| {
            let receipt_map =
                select_edus_receipts_room(&room_id, since_sn, max_edu_sn, &mut num).ok()?;

            receipt_map
                .read
                .is_empty()
                .eq(&false)
                .then_some((room_id, receipt_map))
        })
        .collect();

    if receipts.is_empty() {
        return Ok(None);
    }

    let receipt_content = Edu::Receipt(ReceiptContent::new(receipts));

    let mut buf = EduBuf::new();
    serde_json::to_writer(&mut buf, &receipt_content)
        .expect("Failed to serialize Receipt EDU to JSON vec");

    Ok(Some(buf))
}
/// Look for read receipts in this room
#[tracing::instrument(level = "trace", skip(since_sn))]
fn select_edus_receipts_room(
    room_id: &RoomId,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
    num: &mut usize,
) -> AppResult<ReceiptMap> {
    let receipts = data::room::receipt::read_receipts(room_id, since_sn)?;

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
fn select_edus_presence(
    server_name: &ServerName,
    since_sn: Seqnum,
    _max_edu_sn: &Seqnum,
) -> AppResult<Option<EduBuf>> {
    let presences_since = crate::data::user::presences_since(since_sn)?;

    let mut presence_updates = HashMap::<OwnedUserId, PresenceUpdate>::new();
    for (user_id, presence_event) in presences_since {
        // max_edu_sn.fetch_max(occur_sn, Ordering::Relaxed);
        if !user_id.is_local() {
            continue;
        }

        if !state::server_can_see_user(server_name, &user_id)? {
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
    serde_json::to_writer(&mut buf, &presence_content)
        .expect("failed to serialize Presence EDU to JSON");

    Ok(Some(buf))
}

#[tracing::instrument(skip(server_name))]
pub fn select_edus(server_name: &ServerName) -> AppResult<(EduVec, i64)> {
    let max_edu_sn = data::curr_sn()?;
    let conf = crate::config::get();

    let since_sn = data::curr_sn()?;

    let events_len = AtomicUsize::default();
    let device_changes =
        select_edus_device_changes(server_name, since_sn, &max_edu_sn, &events_len)?;

    let mut events = device_changes;
    if conf.read_receipt.allow_outgoing
        && let Some(receipts) = select_edus_receipts(server_name, since_sn, &max_edu_sn)?
    {
        events.push(receipts);
    }

    if conf.presence.allow_outgoing
        && let Some(presence) = select_edus_presence(server_name, since_sn, &max_edu_sn)?
    {
        events.push(presence);
    }

    Ok((events, max_edu_sn))
}
