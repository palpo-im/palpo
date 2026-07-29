use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use salvo::prelude::*;

use crate::core::client::membership::{
    MutualRoomsReqArgs, MutualRoomsResBody, MutualRoomsV1ReqArgs, MutualRoomsV1ResBody,
};
use crate::core::{MatrixError, OwnedRoomId, RoomId, UserId};
use crate::{AppResult, AuthArgs, DepotExt, JsonResult, data, json_ok};

const MUTUAL_ROOMS_PAGE_SIZE: usize = 100;

async fn mutual_room_ids(
    authenticated_user: &UserId,
    target_user: &UserId,
) -> AppResult<Vec<OwnedRoomId>> {
    let our_rooms: HashSet<_> = data::user::joined_rooms(authenticated_user)
        .await?
        .into_iter()
        .collect();
    let their_rooms = data::user::joined_rooms(target_user).await?;

    Ok(their_rooms
        .into_iter()
        .filter(|room_id| our_rooms.contains(room_id))
        .collect())
}

fn encode_cursor(room_id: &RoomId) -> String {
    URL_SAFE_NO_PAD.encode(room_id.as_str())
}

fn decode_cursor(cursor: &str) -> AppResult<OwnedRoomId> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| MatrixError::invalid_param("Invalid `from` token"))?;
    let room_id =
        String::from_utf8(bytes).map_err(|_| MatrixError::invalid_param("Invalid `from` token"))?;
    RoomId::parse(room_id).map_err(|_| MatrixError::invalid_param("Invalid `from` token").into())
}

fn paginate_mutual_rooms(
    mut joined: Vec<OwnedRoomId>,
    from: Option<&str>,
) -> AppResult<(u64, Vec<OwnedRoomId>, Option<String>)> {
    joined.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
    joined.dedup();

    let count = joined.len() as u64;
    let start = if let Some(cursor) = from {
        let cursor = decode_cursor(cursor)?;
        joined.partition_point(|room_id| room_id.as_str() <= cursor.as_str())
    } else {
        0
    };
    let end = start
        .saturating_add(MUTUAL_ROOMS_PAGE_SIZE)
        .min(joined.len());
    let page = joined[start..end].to_vec();
    let next_batch =
        (end < joined.len()).then(|| encode_cursor(page.last().expect("non-empty page")));

    Ok((count, page, next_batch))
}

/// Get a list of rooms that the authenticated user and another user are both
/// members of.
///
/// This implements MSC2666: Get rooms in common with another user.
#[endpoint]
pub(super) async fn get_mutual_rooms(
    _aa: AuthArgs,
    args: MutualRoomsReqArgs,
    depot: &mut Depot,
) -> JsonResult<MutualRoomsResBody> {
    let authed = depot.authed_info()?;
    let joined = mutual_room_ids(authed.user_id(), &args.user_id).await?;
    let (_, joined, next_batch) = paginate_mutual_rooms(joined, args.batch_token.as_deref())?;

    json_ok(match next_batch {
        Some(token) => MutualRoomsResBody::with_token(joined, token),
        None => MutualRoomsResBody::new(joined),
    })
}

/// Get a paginated list of rooms shared by the authenticated user and another user.
///
/// This is the stable Matrix v1.19 form of MSC2666.
#[endpoint]
pub(super) async fn get_mutual_rooms_v1(
    _aa: AuthArgs,
    args: MutualRoomsV1ReqArgs,
    depot: &mut Depot,
) -> JsonResult<MutualRoomsV1ResBody> {
    let authed = depot.authed_info()?;
    let joined = mutual_room_ids(authed.user_id(), &args.user_id).await?;
    let (count, joined, next_batch) = paginate_mutual_rooms(joined, args.from.as_deref())?;

    json_ok(match next_batch {
        Some(token) => MutualRoomsV1ResBody::with_token(count, joined, token),
        None => MutualRoomsV1ResBody::new(count, joined),
    })
}

#[cfg(test)]
mod tests {
    use super::{MUTUAL_ROOMS_PAGE_SIZE, encode_cursor, paginate_mutual_rooms};
    use crate::core::{OwnedRoomId, owned_room_id};

    fn room(number: usize) -> OwnedRoomId {
        OwnedRoomId::try_from(format!("!room{number:03}:example.org")).unwrap()
    }

    #[test]
    fn stable_pagination_is_sorted_and_restart_safe() {
        let rooms = (0..MUTUAL_ROOMS_PAGE_SIZE + 2).rev().map(room).collect();
        let (count, first_page, next_batch) = paginate_mutual_rooms(rooms, None).unwrap();

        assert_eq!(count, (MUTUAL_ROOMS_PAGE_SIZE + 2) as u64);
        assert_eq!(first_page.len(), MUTUAL_ROOMS_PAGE_SIZE);
        assert_eq!(first_page.first().unwrap(), "!room000:example.org");
        assert_eq!(first_page.last().unwrap(), "!room099:example.org");

        let restarted_rooms = (0..MUTUAL_ROOMS_PAGE_SIZE + 2).map(room).collect();
        let (count, second_page, next_batch) =
            paginate_mutual_rooms(restarted_rooms, next_batch.as_deref()).unwrap();

        assert_eq!(count, (MUTUAL_ROOMS_PAGE_SIZE + 2) as u64);
        assert_eq!(second_page, vec![room(100), room(101)]);
        assert!(next_batch.is_none());
    }

    #[test]
    fn empty_and_single_page_results_do_not_return_a_cursor() {
        let (count, page, next_batch) = paginate_mutual_rooms(Vec::new(), None).unwrap();
        assert_eq!(count, 0);
        assert!(page.is_empty());
        assert!(next_batch.is_none());

        let (count, page, next_batch) =
            paginate_mutual_rooms(vec![room(0), room(0)], None).unwrap();
        assert_eq!(count, 1);
        assert_eq!(page, vec![room(0)]);
        assert!(next_batch.is_none());
    }

    #[test]
    fn cursor_survives_membership_changes_without_repeating_rooms() {
        let cursor = encode_cursor(&room(2));
        let rooms = vec![room(0), room(1), room(3), room(4)];
        let (_, page, _) = paginate_mutual_rooms(rooms, Some(&cursor)).unwrap();

        assert_eq!(page, vec![room(3), room(4)]);
    }

    #[test]
    fn invalid_cursor_is_rejected() {
        let error = paginate_mutual_rooms(vec![owned_room_id!("!room:example.org")], Some("%%%"))
            .unwrap_err();

        assert!(error.to_string().contains("Invalid `from` token"));
    }
}
