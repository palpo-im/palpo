use std::collections::{HashMap, HashSet};

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
use crate::{AppResult, AuthArgs, DepotExt, JsonResult, data, hoops, json_ok, room};

pub fn authed_router() -> Router {
    Router::with_path("user_directory/search")
        .hoop(hoops::limit_rate_user_directory)
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
    json_ok(search_users(authed.user_id(), &body.search_term, body.limit).await?)
}

const SEARCH_BATCH_SIZE: usize = 128;

#[derive(diesel::QueryableByName)]
struct DirectoryProfile {
    #[diesel(sql_type = diesel::sql_types::Text)]
    user_id: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    display_name: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    avatar_url: Option<String>,
}

#[derive(diesel::QueryableByName)]
struct DirectoryMembership {
    #[diesel(sql_type = diesel::sql_types::Text)]
    user_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    room_id: String,
}

async fn search_users(sender: &UserId, term: &str, limit: usize) -> AppResult<SearchUsersResBody> {
    let sender_rooms: HashSet<_> = data::user::joined_rooms(sender)
        .await?
        .into_iter()
        .collect();
    let mut public_rooms = HashMap::new();
    let mut results = Vec::new();
    let mut after = String::new();
    loop {
        // Global profiles only, with stable keyset pagination. A room-specific
        // profile must not duplicate a user or supply their directory identity.
        let profiles = diesel::sql_query(
            "SELECT user_id, display_name, avatar_url FROM (
                SELECT DISTINCT ON (user_id) user_id, display_name, avatar_url
                FROM user_profiles WHERE room_id IS NULL ORDER BY user_id, id DESC
            ) AS profiles WHERE user_id <> $1 AND user_id > $2
                AND (user_id ILIKE $3 OR display_name ILIKE $3)
            ORDER BY user_id LIMIT $4",
        )
        .bind::<diesel::sql_types::Text, _>(sender.as_str())
        .bind::<diesel::sql_types::Text, _>(&after)
        .bind::<diesel::sql_types::Text, _>(format!("%{term}%"))
        .bind::<diesel::sql_types::BigInt, _>(SEARCH_BATCH_SIZE as i64)
        .load::<DirectoryProfile>(&mut connect().await?)
        .await?;
        if profiles.is_empty() {
            return Ok(SearchUsersResBody {
                results,
                limited: false,
            });
        }
        let full_batch = profiles.len() == SEARCH_BATCH_SIZE;
        after = profiles.last().unwrap().user_id.clone();
        let users: Vec<_> = profiles
            .iter()
            .map(|profile| profile.user_id.clone())
            .collect();
        let memberships = diesel::sql_query(
            "SELECT user_id, room_id FROM (
                SELECT DISTINCT ON (user_id, room_id) user_id, room_id, membership
                FROM room_users WHERE user_id = ANY($1)
                ORDER BY user_id, room_id, id DESC
            ) AS current_memberships WHERE membership = 'join'",
        )
        .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(&users)
        .load::<DirectoryMembership>(&mut connect().await?)
        .await?;
        let mut rooms_by_user: HashMap<String, Vec<OwnedRoomId>> = HashMap::new();
        for membership in memberships {
            rooms_by_user
                .entry(membership.user_id)
                .or_default()
                .push(RoomId::parse(membership.room_id)?);
        }
        for profile in profiles {
            let mut visible = false;
            for room_id in rooms_by_user.get(&profile.user_id).into_iter().flatten() {
                if sender_rooms.contains(room_id) {
                    visible = true;
                    break;
                }
                let public = if let Some(public) = public_rooms.get(room_id) {
                    *public
                } else {
                    let public = room::get_state_content::<RoomJoinRulesEventContent>(
                        room_id,
                        &StateEventType::RoomJoinRules,
                        "",
                        None,
                    )
                    .await
                    .map(|rules| rules.join_rule == JoinRule::Public)
                    .unwrap_or(false);
                    public_rooms.insert(room_id.clone(), public);
                    public
                };
                if public {
                    visible = true;
                    break;
                }
            }
            if !visible {
                continue;
            }
            if results.len() >= limit {
                return Ok(SearchUsersResBody {
                    results,
                    limited: true,
                });
            }
            results.push(SearchedUser {
                user_id: UserId::parse(profile.user_id)?,
                display_name: profile.display_name,
                avatar_url: profile.avatar_url.map(OwnedMxcUri::from),
            });
        }
        if !full_batch {
            return Ok(SearchUsersResBody {
                results,
                limited: false,
            });
        }
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use crate::core::UnixMillis;
    use crate::data::schema::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_directory_batches_profiles_and_filters_current_membership() {
        crate::test_database::init();
        let sender: OwnedUserId = "@sender:directory.example".try_into().unwrap();
        for name in (0..140).map(|index| format!("a{index:03}")).chain([
            "z-visible1".into(),
            "z-visible2".into(),
            "z-left".into(),
        ]) {
            let user: OwnedUserId = format!("@{name}:directory.example").try_into().unwrap();
            diesel::insert_into(user_profiles::table)
                .values(data::user::NewDbProfile {
                    user_id: user,
                    room_id: None,
                    display_name: Some("directory fixture".into()),
                    avatar_url: None,
                    blurhash: None,
                })
                .execute(&mut connect().await.unwrap())
                .await
                .unwrap();
        }
        let room: OwnedRoomId = "!shared:directory.example".try_into().unwrap();
        for (index, name, membership) in [
            (1, "sender", "join"),
            (2, "z-visible1", "join"),
            (3, "z-visible2", "join"),
            (4, "z-left", "join"),
            (5, "z-left", "leave"),
        ] {
            let user: OwnedUserId = format!("@{name}:directory.example").try_into().unwrap();
            diesel::insert_into(room_users::table)
                .values(data::room::NewDbRoomUser {
                    event_id: format!("$directory-{index}:directory.example")
                        .try_into()
                        .unwrap(),
                    event_sn: index,
                    room_id: room.clone(),
                    room_server_id: None,
                    user_id: user.clone(),
                    user_server_id: user.server_name().to_owned(),
                    sender_id: sender.clone(),
                    membership: membership.into(),
                    forgotten: false,
                    display_name: None,
                    avatar_url: None,
                    state_data: None,
                    created_at: UnixMillis::now(),
                })
                .execute(&mut connect().await.unwrap())
                .await
                .unwrap();
        }
        diesel::insert_into(user_profiles::table)
            .values(data::user::NewDbProfile {
                user_id: "@z-visible1:directory.example".try_into().unwrap(),
                room_id: Some(room),
                display_name: Some("room-only private name".into()),
                avatar_url: None,
                blurhash: None,
            })
            .execute(&mut connect().await.unwrap())
            .await
            .unwrap();
        let first = search_users(&sender, "directory fixture", 1).await.unwrap();
        assert_eq!(first.results.len(), 1);
        assert!(first.limited);
        assert_eq!(
            first.results[0].user_id.as_str(),
            "@z-visible1:directory.example"
        );
        assert_eq!(
            first.results[0].display_name.as_deref(),
            Some("directory fixture")
        );
        let all = search_users(&sender, "directory fixture", 10)
            .await
            .unwrap();
        assert_eq!(all.results.len(), 2);
        assert!(!all.limited);
        assert!(
            !all.results
                .iter()
                .any(|user| user.user_id.as_str().contains("z-left"))
        );
        assert!(
            search_users(&sender, "room-only", 10)
                .await
                .unwrap()
                .results
                .is_empty()
        );
        assert!(
            search_users(&sender, "' OR true --", 10)
                .await
                .unwrap()
                .results
                .is_empty()
        );
        let zero = search_users(&sender, "directory fixture", 0).await.unwrap();
        assert!(zero.results.is_empty());
        assert!(zero.limited);
    }
}
