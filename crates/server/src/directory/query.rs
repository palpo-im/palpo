//! Request-local directory projection. Only the selected page loads display
//! fields unless those fields are needed by a search. Offset tokens retain the
//! existing static-directory contract; they do not promise a cross-page snapshot.
use std::collections::{BTreeSet, HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Binary, Bool, Json, Nullable, Text};
use diesel_async::RunQueryDsl;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::OwnedRoomId;
use crate::core::directory::{
    PublicRoomFilter, PublicRoomJoinRule, PublicRoomsChunk, PublicRoomsResBody, RoomNetwork,
};
use crate::core::events::room::avatar::RoomAvatarEventContent;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::events::room::create::RoomCreateEventContent;
use crate::core::events::room::guest_access::{GuestAccess, RoomGuestAccessEventContent};
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::name::RoomNameEventContent;
use crate::core::events::room::topic::RoomTopicEventContent;
use crate::core::room::JoinRule;
use crate::room::state::CompressedEvent;
use crate::{AppResult, MatrixError, PduEvent, data};

const TYPES: [&str; 8] = [
    "m.room.create",
    "m.room.join_rules",
    "m.room.name",
    "m.room.topic",
    "m.room.canonical_alias",
    "m.room.history_visibility",
    "m.room.guest_access",
    "m.room.avatar",
];
const CHUNK: usize = 512;

#[derive(QueryableByName)]
struct Candidate {
    #[diesel(sql_type = Text)]
    room_id: OwnedRoomId,
    #[diesel(sql_type = Nullable<BigInt>)]
    frame_id: Option<i64>,
    #[diesel(sql_type = BigInt)]
    joined: i64,
}

#[derive(QueryableByName)]
struct Field {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    event_ty: String,
}

#[derive(QueryableByName)]
struct Delta {
    #[diesel(sql_type = BigInt)]
    frame_id: i64,
    #[diesel(sql_type = Text)]
    room_id: OwnedRoomId,
    #[diesel(sql_type = Nullable<BigInt>)]
    parent_id: Option<i64>,
    #[diesel(sql_type = Binary)]
    appended: Vec<u8>,
    #[diesel(sql_type = Binary)]
    disposed: Vec<u8>,
    #[diesel(sql_type = Bool)]
    valid: bool,
}

#[derive(QueryableByName)]
struct Body {
    #[diesel(sql_type = Text)]
    room_id: OwnedRoomId,
    #[diesel(sql_type = Text)]
    event_id: crate::core::OwnedEventId,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Json)]
    json_data: Value,
}

struct Room {
    chunk: PublicRoomsChunk,
    state: [Option<i64>; 8],
}

type Contents = HashMap<(OwnedRoomId, String), Value>;

fn content<T: DeserializeOwned>(bodies: &Contents, room: &OwnedRoomId, kind: usize) -> Option<T> {
    serde_json::from_value(bodies.get(&(room.clone(), TYPES[kind].into()))?.clone()).ok()
}

// Never install a partial state in STATE_INFO_CACHE. Reconstruct selected
// fields from the same captured frame, rejecting broken/cyclic/foreign chains.
fn state(
    candidate: &Candidate,
    deltas: &HashMap<i64, Delta>,
    fields: &[Field],
) -> Option<[Option<i64>; 8]> {
    let mut frame = candidate.frame_id?;
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(frame) {
            return None;
        }
        let delta = deltas.get(&frame)?;
        if !delta.valid || delta.room_id != candidate.room_id {
            return None;
        }
        chain.push(delta);
        if let Some(parent) = delta.parent_id {
            frame = parent;
        } else {
            break;
        }
    }
    let decode = |bytes: &[u8]| -> Vec<CompressedEvent> {
        bytes
            .chunks_exact(16)
            .map(|item| {
                CompressedEvent::new(
                    i64::from_be_bytes(item[..8].try_into().unwrap()),
                    i64::from_be_bytes(item[8..].try_into().unwrap()),
                )
            })
            .collect()
    };
    let mut full = BTreeSet::new();
    for delta in chain.into_iter().rev() {
        full.extend(decode(&delta.appended));
        // The base frame represents a full state, just as load_frame_info does.
        if delta.parent_id.is_some() {
            for entry in decode(&delta.disposed) {
                full.remove(&entry);
            }
        }
    }
    Some(std::array::from_fn(|kind| {
        let id = fields
            .iter()
            .find(|field| field.event_ty == TYPES[kind])?
            .id;
        full.iter()
            .find(|event| event.field_id() == id)
            .map(CompressedEvent::event_sn)
    }))
}

async fn load_deltas(ids: &[i64], fields: &[Field]) -> AppResult<HashMap<i64, Delta>> {
    let mut result = HashMap::new();
    let prefixes: Vec<Vec<u8>> = fields
        .iter()
        .map(|field| field.id.to_be_bytes().to_vec())
        .collect();
    for ids in ids.chunks(CHUNK) {
        // UNION terminates cycles without an unbounded path array. Projection
        // happens in PostgreSQL: large member states do not cross the wire.
        let rows = diesel::sql_query(
            "WITH RECURSIVE frames AS (
            SELECT * FROM room_state_deltas WHERE frame_id = ANY($1)
            UNION
            SELECT p.* FROM room_state_deltas p JOIN frames c
                ON p.frame_id = c.parent_id AND p.room_id = c.room_id
        ) SELECT frame_id, room_id, parent_id,
            COALESCE((SELECT string_agg(substring(f.appended FROM n FOR 16), ''::bytea ORDER BY n)
                FROM generate_series(1, octet_length(f.appended), 16) n
                WHERE substring(f.appended FROM n FOR 8) = ANY($2)), ''::bytea) AS appended,
            COALESCE((SELECT string_agg(substring(f.disposed FROM n FOR 16), ''::bytea ORDER BY n)
                FROM generate_series(1, octet_length(f.disposed), 16) n
                WHERE substring(f.disposed FROM n FOR 8) = ANY($2)), ''::bytea) AS disposed,
            octet_length(f.appended) % 16 = 0 AND octet_length(f.disposed) % 16 = 0 AS valid
        FROM frames f",
        )
        .bind::<Array<BigInt>, _>(ids)
        .bind::<Array<Binary>, _>(&prefixes)
        .load::<Delta>(&mut data::connect().await?)
        .await?;
        result.extend(rows.into_iter().map(|row| (row.frame_id, row)));
    }
    Ok(result)
}

async fn load_contents(requests: &[(OwnedRoomId, i64, usize)]) -> AppResult<Contents> {
    let mut result = HashMap::new();
    for requests in requests.chunks(CHUNK) {
        let rooms: Vec<_> = requests.iter().map(|(room, ..)| room.as_str()).collect();
        let sns: Vec<_> = requests.iter().map(|(_, sn, _)| *sn).collect();
        let kinds: Vec<_> = requests.iter().map(|(_, _, kind)| TYPES[*kind]).collect();
        let rows = diesel::sql_query("SELECT d.room_id, d.event_id, q.kind, d.json_data
            FROM unnest($1::text[], $2::bigint[], $3::text[]) q(room_id, event_sn, kind)
            JOIN event_points p ON p.room_id = q.room_id AND p.event_sn = q.event_sn
            JOIN events e ON e.id = p.event_id AND e.room_id = q.room_id
            JOIN event_datas d ON d.event_id = p.event_id AND d.room_id = q.room_id AND d.event_sn = q.event_sn")
            .bind::<Array<Text>, _>(&rooms).bind::<Array<BigInt>, _>(&sns)
            .bind::<Array<Text>, _>(&kinds)
            .load::<Body>(&mut data::connect().await?).await?;
        for body in rows {
            if let Ok(pdu) =
                PduEvent::from_json_value(&body.room_id, &body.event_id, body.json_data)
                && pdu.state_key.as_deref() == Some("")
                && pdu.event_ty.to_string() == body.kind
            {
                // State events are exempt from timeline retention. Client-only
                // unsigned metadata is irrelevant to directory content.
                if let Ok(value) = pdu.get_content::<Value>() {
                    result.insert((body.room_id, body.kind), value);
                }
            }
        }
    }
    Ok(result)
}

fn requests(rooms: &[Room], kinds: std::ops::Range<usize>) -> Vec<(OwnedRoomId, i64, usize)> {
    rooms
        .iter()
        .flat_map(|room| {
            kinds.clone().filter_map(|kind| {
                room.state[kind].map(|sn| (room.chunk.room_id.clone(), sn, kind))
            })
        })
        .collect()
}

fn display(chunk: &mut PublicRoomsChunk, bodies: &Contents) {
    let room = &chunk.room_id;
    chunk.name = content::<RoomNameEventContent>(bodies, room, 2).map(|c| c.name);
    chunk.topic = content::<RoomTopicEventContent>(bodies, room, 3).map(|c| c.topic);
    chunk.canonical_alias =
        content::<RoomCanonicalAliasEventContent>(bodies, room, 4).and_then(|c| c.alias);
    chunk.world_readable = content::<RoomHistoryVisibilityEventContent>(bodies, room, 5)
        .is_some_and(|c| c.history_visibility == HistoryVisibility::WorldReadable);
    chunk.guest_can_join = content::<RoomGuestAccessEventContent>(bodies, room, 6)
        .is_some_and(|c| c.guest_access == GuestAccess::CanJoin);
    chunk.avatar_url = content::<RoomAvatarEventContent>(bodies, room, 7).and_then(|c| c.url);
}

pub(super) async fn get_local_public_rooms(
    limit: Option<usize>,
    since: Option<&str>,
    filter: &PublicRoomFilter,
    network: &RoomNetwork,
) -> AppResult<PublicRoomsResBody> {
    if matches!(network, RoomNetwork::ThirdParty(_)) {
        return Ok(PublicRoomsResBody {
            total_room_count_estimate: Some(0),
            ..Default::default()
        });
    }
    let limit = match limit {
        Some(0) => usize::MAX,
        Some(limit) => limit,
        None => 10,
    };
    let (mut start, page_end) = if let Some(token) = since {
        let n = token
            .get(1..)
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(|| MatrixError::invalid_param("Invalid `since` token."))?;
        match token.as_bytes().first() {
            Some(b'n') => (n, None),
            Some(b'p') => (n.saturating_sub(limit), Some(n)),
            _ => return Err(MatrixError::invalid_param("Invalid `since` token.").into()),
        }
    } else {
        (0, None)
    };
    let candidates = diesel::sql_query("SELECT r.id AS room_id, r.state_frame_id AS frame_id,
        COALESCE(s.joined_members, (SELECT count(*) FROM room_users u WHERE u.room_id = r.id AND u.membership = 'join'))::bigint AS joined
        FROM rooms r LEFT JOIN stats_room_currents s ON s.room_id = r.id WHERE r.is_public")
        .load::<Candidate>(&mut data::connect().await?).await?;
    let fields = diesel::sql_query(
        "SELECT id, event_ty FROM room_state_fields WHERE state_key = '' AND event_ty = ANY($1)",
    )
    .bind::<Array<Text>, _>(&TYPES[..])
    .load::<Field>(&mut data::connect().await?)
    .await?;
    let ids: Vec<_> = candidates.iter().filter_map(|room| room.frame_id).collect();
    let deltas = load_deltas(&ids, &fields).await?;
    let mut rooms: Vec<_> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let state = state(&candidate, &deltas, &fields)?;
            Some(Room {
                chunk: PublicRoomsChunk {
                    room_id: candidate.room_id,
                    num_joined_members: candidate.joined.max(0) as u64,
                    name: None,
                    topic: None,
                    canonical_alias: None,
                    avatar_url: None,
                    world_readable: false,
                    guest_can_join: false,
                    room_type: None,
                    join_rule: PublicRoomJoinRule::Public,
                },
                state,
            })
        })
        .collect();
    let search = filter
        .generic_search_term
        .as_ref()
        .map(|s| s.to_lowercase());
    let core = load_contents(&requests(&rooms, 0..if search.is_some() { 5 } else { 2 })).await?;
    rooms.retain_mut(|room| {
        let Some(create) = content::<RoomCreateEventContent>(&core, &room.chunk.room_id, 0) else {
            return false;
        };
        room.chunk.room_type = create.room_type;
        room.chunk.join_rule =
            match content::<RoomJoinRulesEventContent>(&core, &room.chunk.room_id, 1)
                .map(|c| c.join_rule)
            {
                Some(JoinRule::Public) => PublicRoomJoinRule::Public,
                Some(JoinRule::Knock) => PublicRoomJoinRule::Knock,
                Some(JoinRule::KnockRestricted(_)) => PublicRoomJoinRule::KnockRestricted,
                _ => return false,
            };
        if !filter.room_types.is_empty()
            && !filter
                .room_types
                .iter()
                .any(|ty| ty.as_str() == room.chunk.room_type.as_ref().map(|ty| ty.as_str()))
        {
            return false;
        }
        if let Some(search) = &search {
            display(&mut room.chunk, &core);
            room.chunk
                .name
                .as_ref()
                .is_some_and(|s| s.to_lowercase().contains(search))
                || room
                    .chunk
                    .topic
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(search))
                || room
                    .chunk
                    .canonical_alias
                    .as_ref()
                    .is_some_and(|s| s.as_str().to_lowercase().contains(search))
        } else {
            true
        }
    });
    rooms.sort_by(|a, b| {
        b.chunk
            .num_joined_members
            .cmp(&a.chunk.num_joined_members)
            .then_with(|| a.chunk.room_id.cmp(&b.chunk.room_id))
    });
    let total = rooms.len();
    let end = page_end
        .unwrap_or_else(|| start.saturating_add(limit))
        .min(total);
    start = start.min(end);
    let mut page: Vec<_> = rooms.into_iter().skip(start).take(end - start).collect();
    let mut bodies =
        load_contents(&requests(&page, if search.is_some() { 5 } else { 2 }..8)).await?;
    bodies.extend(core);
    for room in &mut page {
        display(&mut room.chunk, &bodies);
    }
    Ok(PublicRoomsResBody {
        chunk: page.into_iter().map(|r| r.chunk).collect(),
        prev_batch: (start > 0).then(|| format!("p{start}")),
        next_batch: (end < total).then(|| format!("n{end}")),
        total_room_count_estimate: Some(total as u64),
    })
}
