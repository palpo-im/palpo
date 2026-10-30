use palpo_core::Seqnum;
use serde::Deserialize;

use crate::core::Direction;
use crate::core::client::relation::RelationEventsResBody;
use crate::core::events::TimelineEventType;
use crate::core::events::relation::RelationType;
use crate::core::identifiers::*;
use crate::data::room::NewDbEventRelation;
use crate::event::{BatchToken, SnPduEvent};
use crate::room::timeline;
use crate::{AppResult, data};

#[derive(Clone, Debug, Deserialize)]
struct ExtractRelType {
    rel_type: RelationType,
}
#[derive(Clone, Debug, Deserialize)]
struct ExtractRelatesToEventId {
    #[serde(rename = "m.relates_to")]
    relates_to: ExtractRelType,
}

#[tracing::instrument]
pub async fn add_relation(
    room_id: &RoomId,
    event_id: &EventId,
    child_id: &EventId,
    rel_type: Option<RelationType>,
) -> AppResult<()> {
    let (event_sn, event_ty) = crate::event::get_event_sn_and_ty(event_id).await?;
    let (child_sn, child_ty) = crate::event::get_event_sn_and_ty(child_id).await?;
    data::room::add_event_relation(&NewDbEventRelation {
        room_id: room_id.to_owned(),
        event_id: event_id.to_owned(),
        event_sn,
        event_ty,
        child_id: child_id.to_owned(),
        child_sn,
        child_ty,
        rel_type: rel_type.map(|v| v.to_string()),
    })
    .await?;
    Ok(())
}

pub async fn paginate_relations_with_filter(
    user_id: &UserId,
    device_id: Option<&DeviceId>,
    room_id: &RoomId,
    target: &EventId,
    filter_event_type: Option<TimelineEventType>,
    filter_rel_type: Option<RelationType>,
    from: Option<&str>,
    to: Option<&str>,
    limit: Option<usize>,
    recurse: bool,
    dir: Direction,
) -> AppResult<RelationEventsResBody> {
    let prev_batch = from.map(|from| from.to_string());
    let from = from
        .map(|from| from.parse())
        .transpose()?
        .unwrap_or(match dir {
            Direction::Forward => BatchToken::LIVE_MIN,
            Direction::Backward => BatchToken::LIVE_MAX,
        });
    let to: Option<BatchToken> = to.map(|to| to.parse()).transpose()?;

    // Use limit or else 10, with maximum 100
    let limit = limit
        .and_then(|u| u32::try_from(u).ok())
        .map_or(10_usize, |u| u as usize)
        .min(100);

    // Spec (v1.10) recommends depth of at least 3
    let depth: u8 = if recurse { 3 } else { 1 };

    let publication = timeline::publication::uses_publication(from)
        || to.is_some_and(timeline::publication::uses_publication);
    let (events, next_token) = if publication {
        get_publication_relations(
            user_id,
            room_id,
            target,
            filter_event_type.as_ref(),
            filter_rel_type.as_ref(),
            from,
            to,
            dir,
            limit,
        )
        .await?
    } else {
        let events: Vec<_> = crate::room::pdu_metadata::get_relations(
            user_id,
            room_id,
            target,
            filter_event_type.as_ref(),
            filter_rel_type.as_ref(),
            from.event_sn(),
            to.map(|t| t.event_sn()),
            dir,
            limit,
        )
        .await?;

        let next_token = match dir {
            Direction::Forward => events
                .last()
                .map(|(_, pdu)| BatchToken::new_live(pdu.event_sn + 1)),
            Direction::Backward => events
                .last()
                .map(|(_, pdu)| BatchToken::new_live(pdu.event_sn - 1)),
        };
        (events, next_token)
    };

    let events: Vec<_> = events
        .into_iter()
        .map(|(_, pdu)| pdu.to_message_like_event_for(user_id, device_id))
        .collect();

    Ok(RelationEventsResBody {
        chunk: events,
        next_batch: next_token.map(|t| t.to_string()),
        prev_batch,
        recursion_depth: if recurse { Some(depth.into()) } else { None },
    })
}

pub async fn get_relations(
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
    child_ty: Option<&TimelineEventType>,
    rel_type: Option<&RelationType>,
    from: Seqnum,
    to: Option<Seqnum>,
    dir: Direction,
    limit: usize,
) -> AppResult<Vec<(Seqnum, SnPduEvent)>> {
    let child_ty = child_ty.map(|t| t.to_string());
    let rel_type = rel_type.map(|t| t.to_string());
    let relations = data::room::get_event_relations(
        room_id,
        event_id,
        child_ty.as_deref(),
        rel_type.as_deref(),
        from,
        to,
        matches!(dir, Direction::Forward),
        limit,
    )
    .await?;
    let mut pdus = Vec::with_capacity(relations.len());
    for relation in relations {
        if let Ok(mut pdu) = timeline::get_pdu(&relation.child_id).await {
            if pdu.sender != user_id {
                pdu.remove_sender_only_unsigned()?;
            }
            if pdu.user_can_see(user_id).await.unwrap_or(false) {
                pdus.push((relation.child_sn, pdu));
            }
        }
    }
    Ok(pdus)
}

/// Legacy relation tokens name absolute child identities with inclusive
/// endpoints. Keep that axis distinct from a /messages q page's signed stream.
fn relation_bound(
    token: BatchToken,
    from: bool,
    direction: Direction,
    window: &mut timeline::publication::Window,
) -> AppResult<Option<BatchToken>> {
    if let BatchToken::Live { stream_ordering } = token {
        let identity = stream_ordering.checked_abs().ok_or_else(|| {
            crate::MatrixError::invalid_param("relation identity is out of range")
        })?;
        if (direction == Direction::Forward) == from {
            window.child_lower = window.child_lower.into_iter().chain(Some(identity)).max();
        } else {
            // MAX is an inclusive last identity, hence no finite exclusive cap.
            window.child_upper = window
                .child_upper
                .into_iter()
                .chain(identity.checked_add(1))
                .min();
        }
        Ok(None)
    } else {
        timeline::publication::bound(token, from, direction, window)
    }
}

async fn get_publication_relations(
    user_id: &UserId,
    room_id: &RoomId,
    target: &EventId,
    child_ty: Option<&TimelineEventType>,
    rel_type: Option<&RelationType>,
    from: BatchToken,
    to: Option<BatchToken>,
    dir: Direction,
    limit: usize,
) -> AppResult<(Vec<(Seqnum, SnPduEvent)>, Option<BatchToken>)> {
    let mut window = timeline::publication::Window::default();
    let from = relation_bound(from, true, dir, &mut window)?;
    let to = to
        .map(|token| relation_bound(token, false, dir, &mut window))
        .transpose()?
        .flatten();
    let seek = |token: BatchToken| {
        (
            token
                .topological_ordering()
                .expect("normalized historic relation cursor"),
            token.stream_ordering(),
        )
    };
    let child_ty = child_ty.map(ToString::to_string);
    let rel_type = rel_type.map(ToString::to_string);
    let relations = data::room::get_event_relations_publication(
        room_id,
        target,
        child_ty.as_deref(),
        rel_type.as_deref(),
        data::room::RelationPublicationWindow {
            lower: window.lower,
            upper: window.upper,
            identity_lower: window.identity_lower,
            identity_upper: window.identity_upper,
            child_lower: window.child_lower,
            child_upper: window.child_upper,
            from: from.map(seek),
            to: to.map(seek),
        },
        matches!(dir, Direction::Forward),
        limit,
    )
    .await?;
    let mut events = Vec::with_capacity(relations.len());
    for relation in relations {
        let mut pdu = match timeline::get_pdu(&relation.child_id).await {
            Ok(pdu) => pdu,
            Err(error) if error.is_not_found() => continue,
            Err(error) => return Err(error),
        };
        if !pdu.user_can_see(user_id).await? {
            continue;
        }
        if pdu.sender != user_id {
            pdu.remove_sender_only_unsigned()?;
        }
        events.push((relation.child_sn, pdu));
    }
    let next = events.last().map(|(_, pdu)| {
        let historic = if dir == Direction::Forward {
            pdu.historic_token()
        } else {
            pdu.prev_historic_token()
        };
        BatchToken::new_publication_page_with_bounds(
            window.lower,
            window.upper,
            historic
                .topological_ordering()
                .expect("historic relation cursor"),
            historic.stream_ordering(),
            window.identity_lower,
            window.identity_upper,
            window.child_lower,
            window.child_upper,
        )
    });
    Ok((events, next))
}

#[cfg(test)]
mod publication_tests;

// #[tracing::instrument(skip(room_id, event_ids))]
// pub fn mark_as_referenced(room_id: &RoomId, event_ids: &[OwnedEventId]) -> AppResult<()> {
// for prev in event_ids {
//     let mut key = room_id.as_bytes().to_vec();
//     key.extend_from_slice(prev.as_bytes());
//     self.referencedevents.insert(&key, &[])?;
// }

//     Ok(())
// }

// pub fn is_event_referenced(room_id: &RoomId, event_id: &EventId) -> AppResult<bool> {
// let mut key = room_id.as_bytes().to_vec();
// key.extend_from_slice(event_id.as_bytes());
// Ok(self.referencedevents.get(&key)?.is_some())
// }

#[tracing::instrument(skip(event_id))]
pub async fn mark_event_soft_failed(event_id: &EventId) -> AppResult<()> {
    data::room::set_event_soft_failed(event_id).await?;
    Ok(())
}

pub async fn is_event_soft_failed(event_id: &EventId) -> AppResult<bool> {
    Ok(data::room::is_event_soft_failed(event_id).await?)
}
