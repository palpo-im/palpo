use diesel::prelude::*;
use palpo_core::Seqnum;
use serde::Deserialize;

use crate::AppResult;
use crate::core::Direction;
use crate::core::client::relation::RelationEventsResBody;
use crate::core::events::TimelineEventType;
use crate::core::events::relation::RelationType;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::room::{DbEventRelation, NewDbEventRelation};
use crate::data::schema::*;
use crate::event::{BatchToken, SnPduEvent};
use crate::room::timeline;

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
pub fn add_relation(
    room_id: &RoomId,
    event_id: &EventId,
    child_id: &EventId,
    rel_type: Option<RelationType>,
) -> AppResult<()> {
    let (event_sn, event_ty) = crate::event::get_event_sn_and_ty(event_id)?;
    let (child_sn, child_ty) = crate::event::get_event_sn_and_ty(child_id)?;
    diesel::insert_into(event_relations::table)
        .values(&NewDbEventRelation {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            event_sn,
            event_ty,
            child_id: child_id.to_owned(),
            child_sn,
            child_ty,
            rel_type: rel_type.map(|v| v.to_string()),
        })
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn paginate_relations_with_filter(
    user_id: &UserId,
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
    )?;

    let next_token = match dir {
        Direction::Forward => events
            .last()
            .map(|(_, pdu)| BatchToken::new_live(pdu.event_sn + 1)),
        Direction::Backward => events
            .last()
            .map(|(_, pdu)| BatchToken::new_live(pdu.event_sn - 1)),
    };

    let events: Vec<_> = events
        .into_iter()
        .map(|(_, pdu)| pdu.to_message_like_event())
        .collect();

    Ok(RelationEventsResBody {
        chunk: events,
        next_batch: next_token.map(|t| t.to_string()),
        prev_batch,
        recursion_depth: if recurse { Some(depth.into()) } else { None },
    })
}

pub fn get_relations(
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
    let mut query = event_relations::table
        .filter(event_relations::room_id.eq(room_id))
        .filter(event_relations::event_id.eq(event_id))
        .into_boxed();
    if let Some(child_ty) = child_ty {
        query = query.filter(event_relations::child_ty.eq(child_ty.to_string()));
    }
    if let Some(rel_type) = rel_type {
        query = query.filter(event_relations::rel_type.eq(rel_type.to_string()));
    }
    match dir {
        Direction::Forward => {
            query = query.filter(event_relations::child_sn.ge(from));
            if let Some(to) = to {
                query = query.filter(event_relations::child_sn.le(to));
            }
            query = query.order_by(event_relations::child_sn.asc());
        }
        Direction::Backward => {
            query = query.filter(event_relations::child_sn.le(from));
            if let Some(to) = to {
                query = query.filter(event_relations::child_sn.ge(to));
            }
            query = query.order_by(event_relations::child_sn.desc());
        }
    }
    let relations = query
        .limit(limit as i64)
        .load::<DbEventRelation>(&mut connect()?)?;
    let mut pdus = Vec::with_capacity(relations.len());
    for relation in relations {
        if let Ok(mut pdu) = timeline::get_pdu(&relation.child_id) {
            if pdu.sender != user_id {
                pdu.remove_transaction_id()?;
            }
            if pdu.user_can_see(user_id).unwrap_or(false) {
                pdus.push((relation.child_sn, pdu));
            }
        }
    }
    Ok(pdus)
}

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
pub fn mark_event_soft_failed(event_id: &EventId) -> AppResult<()> {
    diesel::update(events::table.filter(events::id.eq(event_id)))
        .set(events::soft_failed.eq(true))
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn is_event_soft_failed(event_id: &EventId) -> AppResult<bool> {
    events::table
        .filter(events::id.eq(event_id))
        .select(events::soft_failed)
        .first(&mut connect()?)
        .map_err(Into::into)
}
