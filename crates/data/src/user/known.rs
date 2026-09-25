//! Users this server has seen, on any homeserver.
//!
//! `room_users` keeps the current membership of every user in every room this
//! server participates in, including `leave` and `ban`. Anyone who sent or
//! received events in one of those rooms has a row there, so it is the record
//! of every account the server has seen, local or remote.

use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Nullable, Text};
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Debug, Clone, Default)]
pub struct KnownUsersFilter {
    pub from: i64,
    pub limit: i64,
    /// Case-insensitive substring of the user ID.
    pub search_term: Option<String>,
    /// Exact server name of the user.
    pub server_name: Option<String>,
    /// `Some(true)` keeps users on `local_server_name`, `Some(false)` keeps the
    /// users of every other server.
    pub local: Option<bool>,
    pub local_server_name: String,
    /// `user_id`, `server_name`, `joined_rooms`, `total_rooms` or
    /// `last_membership_at`.
    pub order_by: Option<String>,
    /// `f` for ascending (default), `b` for descending.
    pub dir: Option<String>,
}

#[derive(Debug, Clone, QueryableByName)]
pub struct KnownUserRow {
    #[diesel(sql_type = Text)]
    pub user_id: String,
    #[diesel(sql_type = Text)]
    pub server_name: String,
    #[diesel(sql_type = BigInt)]
    pub joined_rooms: i64,
    #[diesel(sql_type = BigInt)]
    pub invited_rooms: i64,
    #[diesel(sql_type = BigInt)]
    pub left_rooms: i64,
    #[diesel(sql_type = BigInt)]
    pub banned_rooms: i64,
    #[diesel(sql_type = BigInt)]
    pub total_rooms: i64,
    /// When this server last processed a membership change for the user.
    #[diesel(sql_type = BigInt)]
    pub last_membership_at: i64,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// The display name and avatar a user last set in their membership events.
#[derive(Debug, Clone, QueryableByName)]
pub struct KnownUserMemberProfile {
    #[diesel(sql_type = Text)]
    pub user_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub displayname: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub avatar_url: Option<String>,
}

const KNOWN_USERS_WHERE: &str = "\
    WHERE ($1::text IS NULL OR user_id ILIKE $1) \
      AND ($2::text IS NULL OR user_server_id = $2) \
      AND ($3::text IS NULL OR user_server_id = $3) \
      AND ($4::text IS NULL OR user_server_id <> $4)";

/// List the users that appear in `room_users`, one row per user.
pub async fn list_known_users(filter: &KnownUsersFilter) -> DataResult<(Vec<KnownUserRow>, i64)> {
    let search = filter
        .search_term
        .as_deref()
        .filter(|term| !term.is_empty())
        .map(|term| format!("%{}%", super::escape_like_pattern(term)));
    let (local_only, remote_only) = match filter.local {
        Some(true) => (Some(filter.local_server_name.clone()), None),
        Some(false) => (None, Some(filter.local_server_name.clone())),
        None => (None, None),
    };

    let order_col = match filter.order_by.as_deref() {
        Some("server_name") => "server_name",
        Some("joined_rooms") => "joined_rooms",
        Some("total_rooms") => "total_rooms",
        Some("last_membership_at") => "last_membership_at",
        _ => "user_id",
    };
    let order_dir = if filter.dir.as_deref() == Some("b") {
        "DESC"
    } else {
        "ASC"
    };

    let mut conn = connect().await?;
    let count_sql =
        format!("SELECT COUNT(DISTINCT user_id) AS count FROM room_users {KNOWN_USERS_WHERE}");
    let total = diesel::sql_query(count_sql)
        .bind::<Nullable<Text>, _>(&search)
        .bind::<Nullable<Text>, _>(&filter.server_name)
        .bind::<Nullable<Text>, _>(&local_only)
        .bind::<Nullable<Text>, _>(&remote_only)
        .get_result::<CountRow>(&mut conn)
        .await?
        .count;

    // `order_col` and `order_dir` come from the fixed lists above, never from
    // the request, and `user_id` breaks ties so paging is stable.
    let data_sql = format!(
        "SELECT user_id, \
            user_server_id AS server_name, \
            COUNT(*) FILTER (WHERE membership = 'join') AS joined_rooms, \
            COUNT(*) FILTER (WHERE membership = 'invite') AS invited_rooms, \
            COUNT(*) FILTER (WHERE membership = 'leave') AS left_rooms, \
            COUNT(*) FILTER (WHERE membership = 'ban') AS banned_rooms, \
            COUNT(*) AS total_rooms, \
            MAX(created_at) AS last_membership_at \
        FROM room_users {KNOWN_USERS_WHERE} \
        GROUP BY user_id, user_server_id \
        ORDER BY {order_col} {order_dir}, user_id ASC \
        LIMIT $5 OFFSET $6"
    );
    let rows = diesel::sql_query(data_sql)
        .bind::<Nullable<Text>, _>(&search)
        .bind::<Nullable<Text>, _>(&filter.server_name)
        .bind::<Nullable<Text>, _>(&local_only)
        .bind::<Nullable<Text>, _>(&remote_only)
        .bind::<BigInt, _>(filter.limit.max(0))
        .bind::<BigInt, _>(filter.from.max(0))
        .load::<KnownUserRow>(&mut conn)
        .await?;

    Ok((rows, total))
}

/// The profile each user carried in their most recent membership event,
/// preferring rooms they are still joined to.
///
/// Remote users have no global profile row on this server, so this is where
/// their display name and avatar come from.
pub async fn member_profiles(user_ids: &[String]) -> DataResult<Vec<KnownUserMemberProfile>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = diesel::sql_query(
        "SELECT DISTINCT ON (ru.user_id) ru.user_id, \
            ed.json_data -> 'content' ->> 'displayname' AS displayname, \
            ed.json_data -> 'content' ->> 'avatar_url' AS avatar_url \
        FROM room_users ru \
        JOIN event_datas ed ON ed.event_id = ru.event_id \
        WHERE ru.user_id = ANY($1) \
        ORDER BY ru.user_id, (ru.membership = 'join') DESC, ru.event_sn DESC",
    )
    .bind::<Array<Text>, _>(user_ids)
    .load::<KnownUserMemberProfile>(&mut connect().await?)
    .await?;
    Ok(rows)
}

/// One room a known user has a membership in.
#[derive(Debug, Clone)]
pub struct KnownUserRoom {
    pub room_id: OwnedRoomId,
    pub membership: String,
    pub sender_id: OwnedUserId,
    pub event_id: OwnedEventId,
    pub updated_at: UnixMillis,
}

/// Every room the user has a membership in, most recent change first.
pub async fn known_user_rooms(user_id: &UserId) -> DataResult<Vec<KnownUserRoom>> {
    let rows = room_users::table
        .filter(room_users::user_id.eq(user_id))
        .order(room_users::event_sn.desc())
        .select((
            room_users::room_id,
            room_users::membership,
            room_users::sender_id,
            room_users::event_id,
            room_users::created_at,
        ))
        .load::<(OwnedRoomId, String, OwnedUserId, OwnedEventId, UnixMillis)>(&mut connect().await?)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(room_id, membership, sender_id, event_id, updated_at)| KnownUserRoom {
                room_id,
                membership,
                sender_id,
                event_id,
                updated_at,
            },
        )
        .collect())
}
