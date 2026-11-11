//! Regression fixture called by the isolated R08 real-HTTP membership case.
use std::sync::Arc;

use diesel::prelude::*;
use horme::RunQueryDsl;

use crate::core::events::StateEventType;
use crate::core::{OwnedEventId, RoomId, Seqnum, UserId};
use crate::data::schema::*;
use crate::data::{self};
use crate::room::state::{self, CompressedEvent, StateDiff};

async fn publish(room: &RoomId, frame: i64) -> Seqnum {
    let sn = data::next_sn().await.unwrap();
    let id: OwnedEventId = format!("$device-frame-{}", uuid::Uuid::new_v4().simple())
        .parse()
        .unwrap();
    diesel::insert_into(event_points::table)
        .values((
            event_points::event_id.eq(id),
            event_points::event_sn.eq(sn),
            event_points::room_id.eq(room),
            event_points::frame_id.eq(frame),
            event_points::live_published_sn.eq(sn),
            event_points::live_published_frame_id.eq(frame),
        ))
        .execute(&mut crate::data::db())
        .await
        .unwrap();
    sn
}

pub(crate) async fn assert_large_custom_state_is_not_resolved(
    room: &RoomId,
    user: &UserId,
    peer: &UserId,
) {
    let base = crate::room::get_frame_id(room, None).await.unwrap();
    let peer_field = state::get_field_id(&StateEventType::RoomMember, peer.as_str())
        .await
        .unwrap();
    let original_peer = state::load_frame_info(base)
        .await
        .unwrap()
        .last()
        .unwrap()
        .full_state
        .iter()
        .find(|event| event.field_id() == peer_field)
        .copied()
        .unwrap();

    // A partial/imported frame can contain unrelated state whose events are
    // unavailable. They must add neither event-ID lookups nor body reads to
    // device-list sharing. 2,500 fields exercise multiple metadata batches.
    let fields: Vec<_> = (0..2_500)
        .map(|index| {
            (
                room_state_fields::event_ty.eq("org.palpo.unrelated.state"),
                room_state_fields::state_key.eq(format!("unavailable-{index}")),
            )
        })
        .collect();
    let fields: Vec<i64> = diesel::insert_into(room_state_fields::table)
        .values(fields)
        .returning(room_state_fields::id)
        .get_results(&mut crate::data::db())
        .await
        .unwrap();
    let appended: state::CompressedState = fields
        .iter()
        .map(|field| CompressedEvent::new(*field, -1_000_000 - *field))
        .collect();
    let unavailable: Vec<_> = appended.iter().map(CompressedEvent::event_sn).collect();
    assert_eq!(
        event_points::table
            .filter(event_points::event_sn.eq_any(&unavailable))
            .count()
            .get_result::<i64>(&mut crate::data::db())
            .await
            .unwrap(),
        0,
        "unrelated event identities must genuinely be unavailable"
    );
    assert_eq!(
        event_datas::table
            .filter(event_datas::event_sn.eq_any(&unavailable))
            .count()
            .get_result::<i64>(&mut crate::data::db())
            .await
            .unwrap(),
        0,
        "unrelated event bodies must genuinely be unavailable"
    );
    let frame = state::ensure_frame(room, uuid::Uuid::new_v4().as_bytes().to_vec())
        .await
        .unwrap();
    state::save_state_delta(
        room,
        frame,
        StateDiff {
            parent_id: Some(base),
            appended: Arc::new(appended),
            disposed: Arc::new(Default::default()),
        },
    )
    .await
    .unwrap();
    let published = publish(room, frame).await;
    let initial = super::changes(user, 0, published + 1).await.unwrap();
    assert!(initial.changed.iter().any(|id| id == peer));
    assert!(initial.left.is_empty());
    let unchanged = super::changes(user, published + 1, published + 2)
        .await
        .unwrap();
    assert!(unchanged.changed.is_empty());
    assert!(unchanged.left.is_empty());

    // In contrast, a missing selected member cannot silently downgrade to an
    // empty sharing set. Retain the same large frame and replace just its peer
    // membership pointer with a genuinely unavailable body.
    let missing = state::ensure_frame(room, uuid::Uuid::new_v4().as_bytes().to_vec())
        .await
        .unwrap();
    state::save_state_delta(
        room,
        missing,
        StateDiff {
            parent_id: Some(frame),
            appended: Arc::new([CompressedEvent::new(peer_field, Seqnum::MIN / 2)].into()),
            disposed: Arc::new([original_peer].into()),
        },
    )
    .await
    .unwrap();
    let missing_published = publish(room, missing).await;
    let error = match super::changes(user, published + 1, missing_published + 1).await {
        Err(error) => error,
        Ok(_) => panic!("a missing selected member must fail closed"),
    };
    assert!(
        error.to_string().contains("published member event missing"),
        "{error}"
    );
}
