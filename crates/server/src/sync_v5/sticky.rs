//! Bounded MSC4480 delivery, acknowledged by the complete sync response token.
use super::*;
use crate::core::client::sync_events::v3::Sticky;
use crate::core::serde::RawJson;
use crate::event::{STICKY_TTL_KEY, SnPduEvent};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Delivery {
    config: Vec<u8>,
    generation: u64,
    rooms: BTreeMap<OwnedRoomId, Progress>,
    offers: BTreeMap<String, Offer>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Progress {
    join: Seqnum,
    next: Seqnum,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Offer {
    input: Option<String>,
    generation: u64,
    progress: BTreeMap<OwnedRoomId, Progress>,
    entries: BTreeMap<OwnedRoomId, Vec<OwnedEventId>>,
    #[serde(skip)]
    pub(super) senders: BTreeMap<OwnedRoomId, BTreeSet<OwnedUserId>>,
}

fn config_key(request: &SyncEventsReqBody) -> Vec<u8> {
    let config = serde_json::to_vec(&(
        &request.extensions.sticky_events,
        &request.lists,
        &request.room_subscriptions,
    ))
    .expect("sticky config serializes");
    crate::utils::hash_keys(std::iter::once(config))
}
pub(super) fn configure(state: &mut Delivery, request: &SyncEventsReqBody) {
    let config = config_key(request);
    if state.config != config {
        state.config = config;
        state.generation = state.generation.saturating_add(1);
        state.offers.clear();
    }
    if !request.extensions.sticky_events.enabled {
        state.rooms.clear();
    }
}

pub(super) async fn acknowledge(
    user: &UserId,
    device: &DeviceId,
    request: &SyncEventsReqBody,
    token: Option<&str>,
) -> AppResult<()> {
    if !request.extensions.sticky_events.enabled {
        return Ok(());
    }
    let accepted = update_required_state_cache(user, device, &request.conn_id, |cached| {
        let state = &mut cached.sticky_delivery;
        if state.config != config_key(request) {
            return false;
        }
        if let Some(offer) = token.and_then(|token| state.offers.get(token)).cloned()
            && offer.generation == state.generation
        {
            for (room, progress) in offer.progress {
                match state.rooms.get_mut(&room) {
                    Some(previous) if previous.join == progress.join => {
                        previous.next = previous.next.max(progress.next)
                    }
                    _ => {
                        state.rooms.insert(room, progress);
                    }
                }
            }
        }
        true
    })
    .await?;
    if !accepted {
        return Err(crate::AppError::internal(
            "sticky sync request configuration is stale",
        ));
    }
    Ok(())
}

fn scope(
    config: &StickyEventsConfig,
    lists: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    request: &SyncEventsReqBody,
) -> BTreeSet<OwnedRoomId> {
    let mut selected = BTreeSet::new();
    for (id, rooms) in lists {
        if config
            .lists
            .as_ref()
            .is_none_or(|ids| ids.iter().any(|s| s == "*" || s == id))
        {
            selected.extend(rooms.iter().cloned());
        }
    }
    match &config.rooms {
        None => selected.extend(request.room_subscriptions.keys().cloned()),
        Some(rooms) => {
            for room in rooms {
                match room {
                    ExtensionRoomConfig::AllSubscribed => {
                        selected.extend(request.room_subscriptions.keys().cloned())
                    }
                    ExtensionRoomConfig::Room(id) => {
                        selected.insert(id.clone());
                    }
                }
            }
        }
    }
    selected
}

async fn visible_pdu(user: &UserId, id: &EventId) -> AppResult<Option<(SnPduEvent, UnixMillis)>> {
    let expiry = event_stickies::table
        .find(id)
        .filter(event_stickies::expires_at.gt(UnixMillis::now().0 as i64))
        .filter(event_stickies::deliver_sn.is_not_null())
        .select(event_stickies::expires_at)
        .first::<i64>(&mut data::db())
        .await
        .optional()?;
    let Some(expiry) = expiry else {
        return Ok(None);
    };
    let pdu = match room::timeline::get_pdu(id).await {
        Ok(pdu) => pdu,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error),
    };
    if pdu.sticky_duration_ms().is_none() || !room::user::is_joined(user, &pdu.room_id).await? {
        return Ok(None);
    }
    if crate::user::ignored_users(user).await.contains(&pdu.sender)
        || pdu.event_ty.to_string() == "org.matrix.dummy_event"
    {
        return Ok(None);
    }
    #[cfg(feature = "unstable-msc1763")]
    if crate::retention::is_expired(
        &pdu.event_id,
        &pdu.room_id,
        pdu.state_key.as_deref(),
        pdu.origin_server_ts,
    )
    .await?
    {
        return Ok(None);
    }
    Ok(Some((pdu, UnixMillis(expiry as u64))))
}

pub(super) async fn collect(
    info: SyncInfo<'_>,
    input: Option<&str>,
    lists: &BTreeMap<String, BTreeSet<OwnedRoomId>>,
    todo: &mut TodoRooms,
    until: Seqnum,
) -> AppResult<Offer> {
    let SyncInfo {
        sender_id: user,
        device_id: device,
        req_body: request,
        ..
    } = info;
    let config = &request.extensions.sticky_events;
    if !config.enabled {
        return Ok(Offer::default());
    }
    let limit = config.limit.unwrap_or(100).min(100) as i64;
    if limit == 0 {
        return Err(
            crate::MatrixError::invalid_param("sticky event limit must be positive").into(),
        );
    }
    let mut joins = BTreeMap::new();
    for room_id in scope(config, lists, request) {
        if room::user::is_joined(user, &room_id).await? {
            let member =
                room::get_state(&room_id, &StateEventType::RoomMember, user.as_str(), None).await?;
            let join =
                room::user::join_sn_for_event(user, member.event_id.clone(), member.event_sn)
                    .await?;
            joins.insert(room_id, join);
        }
    }
    let state = update_required_state_cache(user, device, &request.conn_id, |cached| {
        let state = &mut cached.sticky_delivery;
        if state.config != config_key(request) {
            return None;
        }
        state
            .rooms
            .retain(|id, progress| joins.get(id) == Some(&progress.join));
        state.offers.retain(|_, offer| {
            offer.progress.len() == joins.len()
                && offer
                    .progress
                    .iter()
                    .all(|(id, p)| joins.get(id) == Some(&p.join))
        });
        Some(state.clone())
    })
    .await?
    .ok_or_else(|| crate::AppError::internal("sticky sync request configuration is stale"))?;
    let mut offer = if let Some(previous) = state.offers.values().find(|o| {
        o.input.as_deref() == input
            // An empty probe built before long polling has not been returned.
            && o.entries.values().any(|ids| !ids.is_empty())
            && o.generation == state.generation
            && o.progress.len() == joins.len()
            && o.progress
                .iter()
                .all(|(id, p)| joins.get(id) == Some(&p.join))
    }) {
        previous.clone()
    } else {
        let mut offer = Offer {
            input: input.map(str::to_owned),
            generation: state.generation,
            ..Default::default()
        };
        let ignored: Vec<_> = crate::user::ignored_users(user).await.into_iter().collect();
        for (room_id, join) in joins {
            let start = state.rooms.get(&room_id).map(|p| p.next).unwrap_or(0);
            let page = event_stickies::table
                .inner_join(events::table.on(events::id.eq(event_stickies::event_id)))
                .filter(event_stickies::room_id.eq(&room_id))
                .filter(event_stickies::deliver_sn.ge(start))
                .filter(event_stickies::deliver_sn.lt(until))
                .filter(event_stickies::expires_at.gt(UnixMillis::now().0 as i64))
                .filter(events::is_outlier.eq(false))
                .filter(events::soft_failed.eq(false))
                .filter(events::is_rejected.eq(false))
                .filter(events::is_redacted.eq(false))
                .filter(events::ty.ne("org.matrix.dummy_event"))
                .filter(
                    events::sender_id
                        .ne_all(&ignored)
                        .or(events::sender_id.is_null()),
                )
                .order(event_stickies::deliver_sn.asc())
                .limit(limit)
                .select((event_stickies::event_id, event_stickies::deliver_sn))
                .load::<(OwnedEventId, Option<Seqnum>)>(&mut data::db())
                .await?;
            let next = if page.len() == limit as usize {
                page.last()
                    .and_then(|(_, sn)| *sn)
                    .unwrap_or(start)
                    .saturating_add(1)
            } else {
                until
            };
            offer.entries.insert(
                room_id.clone(),
                page.into_iter().map(|(id, _)| id).collect(),
            );
            offer.progress.insert(room_id, Progress { join, next });
        }
        offer
    };
    for (room_id, ids) in &offer.entries {
        let mut senders = BTreeSet::new();
        for id in ids {
            if let Some((pdu, _)) = visible_pdu(user, id).await? {
                senders.insert(pdu.sender.clone());
            }
        }
        if let Some(room) = todo.get(room_id)
            && room
                .required_state
                .iter()
                .any(|key| key.0 == StateEventType::RoomMember && key.1 == "$LAZY")
        {
            offer.senders.insert(room_id.clone(), senders);
        }
    }
    Ok(offer)
}

pub(super) async fn render(
    info: SyncInfo<'_>,
    offer: &Offer,
    response: &mut SyncEventsResBody,
) -> AppResult<()> {
    if !info.req_body.extensions.sticky_events.enabled {
        return Ok(());
    }
    // Every timeline copy, including copies outside extension scope, needs the authoritative TTL.
    for room in response.rooms.values_mut() {
        for event in &mut room.timeline {
            let mut value: serde_json::Value = serde_json::from_str(event.as_str())?;
            if value.get(crate::event::STICKY_KEY).is_some()
                && let Some(id) = value["event_id"]
                    .as_str()
                    .and_then(|id| EventId::parse(id).ok())
            {
                let ttl = if let Some((_, expiry)) = visible_pdu(info.sender_id, &id).await? {
                    crate::event::sticky::ttl_ms(expiry, UnixMillis::now())
                } else {
                    0
                };
                value["unsigned"][STICKY_TTL_KEY] = serde_json::json!(ttl);
                *event = RawJson::from_value(&value)?;
            }
        }
    }
    for (room_id, ids) in &offer.entries {
        let mut events = Vec::new();
        let timeline_ids: BTreeSet<String> = response
            .rooms
            .get(room_id)
            .map(|r| {
                r.timeline
                    .iter()
                    .filter_map(|e| {
                        serde_json::from_str::<serde_json::Value>(e.as_str())
                            .ok()?
                            .get("event_id")?
                            .as_str()
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            if timeline_ids.contains(id.as_str()) {
                continue;
            }
            if let Some((mut pdu, expiry)) = visible_pdu(info.sender_id, id).await? {
                if pdu.sender != info.sender_id {
                    pdu.remove_sender_only_unsigned()?;
                }
                pdu.add_unsigned_membership(info.sender_id).await?;
                pdu.load_thread_summary_for(info.sender_id).await;
                pdu.add_age()?;
                events.push(
                    crate::event::sticky::with_ttl(pdu, expiry, UnixMillis::now())
                        .to_sync_room_event_for(info.sender_id, Some(info.device_id)),
                );
            }
        }
        if !events.is_empty() {
            response
                .extensions
                .sticky_events
                .rooms
                .insert(room_id.clone(), Sticky { events });
        }
    }
    Ok(())
}

pub(super) async fn record(
    info: SyncInfo<'_>,
    mut offer: Offer,
    response: &mut SyncEventsResBody,
) -> AppResult<()> {
    if !info.req_body.extensions.sticky_events.enabled {
        return Ok(());
    }
    offer.senders.clear();
    let old = response.pos.clone();
    let base = old
        .split('_')
        .filter(|p| !p.starts_with('w'))
        .collect::<Vec<_>>()
        .join("_");
    let token = format!("{base}_w{}", crate::utils::random_string(24));
    let saved = update_required_state_cache(
        info.sender_id,
        info.device_id,
        &info.req_body.conn_id,
        |cached| {
            if cached.sticky_delivery.generation != offer.generation {
                return false;
            }
            // Concurrent requests with the same input must not publish different
            // pages. The first persisted offer wins; the other request retries it.
            if cached.sticky_delivery.offers.values().any(|previous| {
                previous.input == offer.input
                    && previous.generation == offer.generation
                    && previous.entries.values().any(|ids| !ids.is_empty())
                    && (previous.entries != offer.entries || previous.progress != offer.progress)
            }) {
                return false;
            }
            cached.sticky_delivery.offers.retain(|_, previous| {
                previous.input != offer.input
                    || previous.entries.values().any(|ids| !ids.is_empty())
            });
            if cached.sticky_delivery.offers.len() >= 16 {
                cached.sticky_delivery.offers.clear();
            }
            cached
                .sticky_delivery
                .offers
                .insert(token.clone(), offer.clone());
            if let Some(v) = cached.pending_required_state.remove(&old) {
                cached.pending_required_state.insert(token.clone(), v);
            }
            if let Some(v) = cached.pending_list_windows.remove(&old) {
                cached.pending_list_windows.insert(token.clone(), v);
            }
            true
        },
    )
    .await?;
    if !saved {
        return Err(crate::AppError::internal(
            "sticky sync configuration changed during response",
        ));
    }
    response.pos = token;
    Ok(())
}

#[cfg(test)]
#[path = "sticky_tests.rs"]
mod tests;
