use diesel::prelude::*;
use diesel_async::RunQueryDsl;
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
async fn search(
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
        .load::<OwnedUserId>(&mut connect().await?)
        .await?;

    let mut results = Vec::new();
    let mut limited = false;
    for user_id in user_ids {
        let user = SearchedUser {
            user_id: user_id.clone(),
            display_name: data::user::display_name(&user_id).await.ok().flatten(),
            avatar_url: data::user::avatar_url(&user_id).await.ok().flatten(),
        };

        let matched = 'matched: {
            let Some(joined_rooms) = data::user::joined_rooms(&user_id).await.ok() else {
                break 'matched false;
            };
            let mut user_is_in_public_rooms = false;
            for room_id in joined_rooms.into_iter() {
                if room::get_state_content::<RoomJoinRulesEventContent>(
                    &room_id,
                    &StateEventType::RoomJoinRules,
                    "",
                    None,
                )
                .await
                .map(|r| r.join_rule == JoinRule::Public)
                .unwrap_or(false)
                {
                    user_is_in_public_rooms = true;
                    break;
                }
            }

            if user_is_in_public_rooms {
                break 'matched true;
            }

            let Some(shared_rooms) =
                room::user::shared_rooms(vec![authed.user_id().to_owned(), user_id.clone()])
                    .await
                    .ok()
            else {
                break 'matched false;
            };

            !shared_rooms.is_empty()
        };

        if !matched {
            continue;
        }

        if results.len() >= body.limit {
            limited = true;
            break;
        }
        results.push(user);
    }

    json_ok(SearchUsersResBody { results, limited })
}
