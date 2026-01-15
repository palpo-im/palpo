use diesel::prelude::*;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::user_directory::{
    SearchUsersReqArgs, SearchUsersReqBody, SearchUsersResBody, SearchedUser,
};
use crate::core::events::StateEventType;
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::identifiers::*;
use crate::core::room::JoinRule;
use crate::data::connect;
use crate::data::schema::*;
use crate::{AuthArgs, DepotExt, JsonResult, data, hoops, json_ok, room};

pub fn authed_router() -> Router {
    Router::with_path("user_directory/search")
        .hoop(hoops::limit_rate)
        .post(search)
}

/// #POST /_matrix/client/r0/user_directory/search
/// Searches all known users for a match.
///
/// - Hides any local users that aren't in any public rooms (i.e. those that have the join rule set
///   to public)
/// and don't share a room with the sender
#[endpoint]
fn search(
    _aa: AuthArgs,
    _args: SearchUsersReqArgs,
    body: JsonBody<SearchUsersReqBody>,
    depot: &mut Depot,
) -> JsonResult<SearchUsersResBody> {
    let authed = depot.authed_info()?;
    let body = body.into_inner();
    let user_ids = user_profiles::table
        .filter(
            user_profiles::user_id
                .ilike(format!("%{}%", body.search_term))
                .or(user_profiles::display_name.ilike(format!("%{}%", body.search_term))),
        )
        .filter(user_profiles::user_id.ne(authed.user_id()))
        .select(user_profiles::user_id)
        .load::<OwnedUserId>(&mut connect()?)?;

    let mut users = user_ids.into_iter().filter_map(|user_id| {
        let user = SearchedUser {
            user_id: user_id.clone(),
            display_name: data::user::display_name(&user_id).ok().flatten(),
            avatar_url: data::user::avatar_url(&user_id).ok().flatten(),
        };

        let user_is_in_public_rooms =
            data::user::joined_rooms(&user_id)
                .ok()?
                .into_iter()
                .any(|room_id| {
                    room::get_state_content::<RoomJoinRulesEventContent>(
                        &room_id,
                        &StateEventType::RoomJoinRules,
                        "",
                        None,
                    )
                    .map(|r| r.join_rule == JoinRule::Public)
                    .unwrap_or(false)
                });

        if user_is_in_public_rooms {
            return Some(user);
        }

        let user_is_in_shared_rooms =
            !room::user::shared_rooms(vec![authed.user_id().to_owned(), user_id])
                .ok()?
                .is_empty();

        if user_is_in_shared_rooms {
            return Some(user);
        }

        None
    });

    let results = users.by_ref().take(body.limit).collect();
    let limited = users.next().is_some();
    json_ok(SearchUsersResBody { results, limited })
}
