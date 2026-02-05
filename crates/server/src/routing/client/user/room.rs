use std::collections::HashSet;

use salvo::prelude::*;

use crate::core::client::membership::{MutualRoomsReqArgs, MutualRoomsResBody};
use crate::{AuthArgs, DepotExt, JsonResult, data, json_ok};

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

    // Get the authenticated user's joined rooms
    let our_rooms: HashSet<_> = data::user::joined_rooms(authed.user_id())?
        .into_iter()
        .collect();

    // Get the target user's joined rooms
    let their_rooms = data::user::joined_rooms(&args.user_id)?;

    // Find the intersection (mutual rooms)
    let joined: Vec<_> = their_rooms
        .into_iter()
        .filter(|room_id| our_rooms.contains(room_id))
        .collect();

    json_ok(MutualRoomsResBody::new(joined))
}
