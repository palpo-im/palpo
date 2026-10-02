//! Palpo Admin API - users this server has seen
//!
//! - GET /_palpo/admin/v1/known_users
//! - GET /_palpo/admin/v1/known_users/{user_id}
//!
//! Unlike `/v2/users`, which lists the accounts registered on this server,
//! these endpoints list every user, local or remote, that has a membership in
//! a room this server participates in. They are read-only: a remote account is
//! owned by its own homeserver and cannot be managed here.

use std::collections::HashMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::core::identifiers::*;
use crate::data::user::known::{self, KnownUsersFilter};
use crate::exts::*;
use crate::{JsonResult, MatrixError, config, data, json_ok, room};

pub fn router() -> Router {
    Router::with_path("v1/known_users")
        .get(list_known_users)
        .push(Router::with_path("{user_id}").get(get_known_user))
}

/// Largest page `list_known_users` returns.
const MAX_LIMIT: i64 = 500;

#[derive(Debug, Serialize, ToSchema)]
pub struct KnownUserInfo {
    pub user_id: String,
    pub server_name: String,
    pub is_local: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displayname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub joined_rooms: i64,
    pub invited_rooms: i64,
    pub left_rooms: i64,
    pub banned_rooms: i64,
    pub total_rooms: i64,
    /// When this server last processed a membership change for the user, in
    /// milliseconds since the Unix epoch.
    pub last_membership_ts: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KnownUsersResponse {
    pub users: Vec<KnownUserInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    pub total: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KnownUserRoomInfo {
    pub room_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub membership: String,
    /// The user who sent the membership event (the inviter, kicker or banner
    /// when it is not the user themselves).
    pub sender: String,
    pub event_id: String,
    pub updated_ts: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct KnownUserDetail {
    pub user_id: String,
    pub server_name: String,
    pub is_local: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displayname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Whether the user has an account on this server. Always false for remote
    /// users.
    pub has_account: bool,
    pub rooms: Vec<KnownUserRoomInfo>,
}

/// The display name and avatar to show for each user: the global profile when
/// this server has one (local users), otherwise the one from the user's latest
/// membership event.
async fn resolve_profiles(
    user_ids: &[String],
) -> crate::AppResult<HashMap<String, (Option<String>, Option<String>)>> {
    let mut profiles: HashMap<String, (Option<String>, Option<String>)> =
        known::member_profiles(user_ids)
            .await?
            .into_iter()
            .map(|p| (p.user_id, (p.displayname, p.avatar_url)))
            .collect();

    let owned_ids: Vec<OwnedUserId> = user_ids
        .iter()
        .filter_map(|id| UserId::parse(id).ok())
        .collect();
    for global in data::user::get_global_profiles(&owned_ids).await? {
        let entry = profiles.entry(global.user_id.to_string()).or_default();
        if global.display_name.is_some() {
            entry.0 = global.display_name;
        }
        if let Some(avatar_url) = global.avatar_url {
            entry.1 = Some(avatar_url.to_string());
        }
    }
    Ok(profiles)
}

/// GET /_palpo/admin/v1/known_users
///
/// List every user, local or remote, with a membership in a room this server
/// participates in.
#[endpoint]
pub async fn list_known_users(
    from: QueryParam<i64, false>,
    limit: QueryParam<i64, false>,
    search_term: QueryParam<String, false>,
    server_name: QueryParam<String, false>,
    local: QueryParam<bool, false>,
    order_by: QueryParam<String, false>,
    dir: QueryParam<String, false>,
) -> JsonResult<KnownUsersResponse> {
    let from = from.into_inner().unwrap_or(0).max(0);
    let limit = limit.into_inner().unwrap_or(100).clamp(1, MAX_LIMIT);
    let filter = KnownUsersFilter {
        from,
        limit,
        search_term: search_term.into_inner(),
        server_name: server_name.into_inner().filter(|s| !s.is_empty()),
        local: local.into_inner(),
        local_server_name: config::get().server_name.to_string(),
        order_by: order_by.into_inner(),
        dir: dir.into_inner(),
    };
    let (rows, total) = known::list_known_users(&filter).await?;

    let user_ids: Vec<String> = rows.iter().map(|r| r.user_id.clone()).collect();
    let mut profiles = resolve_profiles(&user_ids).await?;
    let local_server = &config::get().server_name;

    let users: Vec<KnownUserInfo> = rows
        .into_iter()
        .map(|row| {
            let (displayname, avatar_url) = profiles.remove(&row.user_id).unwrap_or_default();
            KnownUserInfo {
                is_local: row.server_name == local_server.as_str(),
                user_id: row.user_id,
                server_name: row.server_name,
                displayname,
                avatar_url,
                joined_rooms: row.joined_rooms,
                invited_rooms: row.invited_rooms,
                left_rooms: row.left_rooms,
                banned_rooms: row.banned_rooms,
                total_rooms: row.total_rooms,
                last_membership_ts: row.last_membership_at,
            }
        })
        .collect();

    let next = from + users.len() as i64;
    let next_token = (next < total).then(|| next.to_string());
    json_ok(KnownUsersResponse {
        users,
        next_token,
        total,
    })
}

/// GET /_palpo/admin/v1/known_users/{user_id}
///
/// Show a known user and every room they have a membership in.
#[endpoint]
pub async fn get_known_user(user_id: PathParam<OwnedUserId>) -> JsonResult<KnownUserDetail> {
    let user_id = user_id.into_inner();
    let memberships = known::known_user_rooms(&user_id).await?;
    if memberships.is_empty() {
        return Err(MatrixError::not_found("This server has not seen the user in any room").into());
    }

    let mut profiles = resolve_profiles(&[user_id.to_string()]).await?;
    let (displayname, avatar_url) = profiles.remove(user_id.as_str()).unwrap_or_default();
    let is_local = user_id.is_local();
    let has_account = is_local && data::user::user_exists(&user_id).await?;

    let mut rooms = Vec::with_capacity(memberships.len());
    for membership in memberships {
        let name = room::get_name(&membership.room_id)
            .await
            .ok()
            .filter(|n| !n.is_empty());
        rooms.push(KnownUserRoomInfo {
            room_id: membership.room_id.to_string(),
            name,
            membership: membership.membership,
            sender: membership.sender_id.to_string(),
            event_id: membership.event_id.to_string(),
            updated_ts: membership.updated_at.get() as i64,
        });
    }

    json_ok(KnownUserDetail {
        server_name: user_id.server_name().to_string(),
        user_id: user_id.to_string(),
        is_local,
        displayname,
        avatar_url,
        has_account,
        rooms,
    })
}

#[cfg(test)]
mod tests {
    use diesel::sql_types::{BigInt, Text};
    use diesel_async::RunQueryDsl;
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use crate::core::identifiers::*;
    use crate::core::{UnixMillis, user_id};
    use crate::data::{self, connect};

    async fn fixture_account(user_id: &UserId, admin: bool, token: Option<&str>) {
        data::user::create_user(&data::user::NewDbUser {
            id: user_id.to_owned(),
            ty: None,
            is_admin: admin,
            is_guest: false,
            is_local: true,
            localpart: user_id.localpart().to_owned(),
            server_name: user_id.server_name().to_owned(),
            appservice_id: None,
            created_at: UnixMillis::now(),
        })
        .await
        .unwrap();
        if let Some(token) = token {
            let device_id: OwnedDeviceId = "KNOWN_USERS_DEVICE".into();
            data::user::device::create_device(user_id, &device_id, token, None, None)
                .await
                .unwrap();
        }
    }

    async fn fixture_membership(
        room_id: &str,
        user_id: &str,
        membership: &str,
        displayname: Option<&str>,
        event_sn: i64,
    ) {
        let event_id = format!("$known-{event_sn}");
        let server = user_id.split_once(':').unwrap().1;
        let mut conn = connect().await.unwrap();
        diesel::sql_query(
            "INSERT INTO room_users (event_id, event_sn, room_id, user_id, user_server_id, \
                sender_id, membership, created_at) \
             VALUES ($1, $2, $3, $4, $5, $4, $6, $2)",
        )
        .bind::<Text, _>(&event_id)
        .bind::<BigInt, _>(event_sn)
        .bind::<Text, _>(room_id)
        .bind::<Text, _>(user_id)
        .bind::<Text, _>(server)
        .bind::<Text, _>(membership)
        .execute(&mut conn)
        .await
        .unwrap();
        let content = json!({"membership": membership, "displayname": displayname});
        diesel::sql_query(
            "INSERT INTO event_datas (event_id, event_sn, room_id, json_data) \
             VALUES ($1, $2, $3, $4::json)",
        )
        .bind::<Text, _>(&event_id)
        .bind::<BigInt, _>(event_sn)
        .bind::<Text, _>(room_id)
        .bind::<Text, _>(json!({"content": content}).to_string())
        .execute(&mut conn)
        .await
        .unwrap();
    }

    async fn request(
        service: &salvo::Service,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (Option<salvo::http::StatusCode>, Value) {
        let url = format!("http://localhost/_palpo/admin/{path}");
        let request = match method {
            "PUT" => TestClient::put(url),
            "POST" => TestClient::post(url),
            _ => TestClient::get(url),
        };
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let mut response = request
            .add_header("authorization", "Bearer known-users-admin-token", true)
            .send(service)
            .await;
        let status = response.status_code;
        let body = response.take_json::<Value>().await.unwrap_or(Value::Null);
        (status, body)
    }

    fn user_ids(body: &Value) -> Vec<String> {
        body["users"]
            .as_array()
            .unwrap()
            .iter()
            // `/v2/users` calls the ID `name`, `/v1/known_users` calls it `user_id`.
            .map(|u| u.get("user_id").unwrap_or(&u["name"]).as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL"]
    async fn database_admin_user_endpoints_separate_local_accounts_from_known_users() {
        use salvo::http::StatusCode;

        crate::test_database::init();
        crate::config::CONFIG.get_or_init(|| {
            serde_json::from_value(json!({
                "server_name": "known-users.example", "db": {"url": "unused-test-config"},
            }))
            .unwrap()
        });
        let local = crate::config::get().server_name.to_string();
        let service = salvo::Service::new(crate::routing::admin::router());

        let admin = UserId::parse(format!("@kuadmin:{local}")).unwrap();
        let alice = UserId::parse(format!("@kualice:{local}")).unwrap();
        fixture_account(&admin, true, Some("known-users-admin-token")).await;
        fixture_account(&alice, false, None).await;
        // A leftover row for another server, e.g. from before a server_name change.
        fixture_account(user_id!("@kughost:elsewhere.example"), false, None).await;

        // The account list only shows accounts on this server.
        let (status, body) = request(&service, "GET", "v2/users?name=ku", None).await;
        assert_eq!(status, Some(StatusCode::OK));
        assert_eq!(user_ids(&body), vec![admin.to_string(), alice.to_string()]);
        assert_eq!(body["total"], 2);

        // Remote IDs are refused instead of creating or changing a placeholder row.
        for (method, path, body) in [
            (
                "PUT",
                "v2/users/@kubob:remote.example",
                json!({"deactivated": true}),
            ),
            (
                "POST",
                "v1/deactivate/@kughost:elsewhere.example",
                json!({"erase": false}),
            ),
            (
                "PUT",
                "v1/users/@kughost:elsewhere.example/admin",
                json!({"admin": true}),
            ),
        ] {
            let (status, _) = request(&service, method, path, Some(body)).await;
            assert_eq!(status, Some(StatusCode::BAD_REQUEST), "{method} {path}");
        }
        assert!(
            !data::user::user_exists(user_id!("@kubob:remote.example"))
                .await
                .unwrap()
        );
        assert!(
            !data::user::is_deactivated(user_id!("@kughost:elsewhere.example"))
                .await
                .unwrap()
        );

        // Known users come from room memberships, on any server.
        let room = format!("!kuroom:{local}");
        let other_room = format!("!kuother:{local}");
        fixture_membership(&room, alice.as_str(), "join", Some("Alice"), 9_100_001).await;
        fixture_membership(
            &room,
            "@kucarol:remote.example",
            "join",
            Some("Carol"),
            9_100_002,
        )
        .await;
        fixture_membership(
            &other_room,
            "@kucarol:remote.example",
            "leave",
            None,
            9_100_003,
        )
        .await;
        fixture_membership(&room, "@kudave:other.example", "ban", None, 9_100_004).await;

        let (status, body) = request(&service, "GET", "v1/known_users?search_term=ku", None).await;
        assert_eq!(status, Some(StatusCode::OK));
        assert_eq!(body["total"], 3);
        assert_eq!(
            user_ids(&body),
            vec![
                alice.to_string(),
                "@kucarol:remote.example".to_owned(),
                "@kudave:other.example".to_owned(),
            ]
        );
        let carol = &body["users"][1];
        assert_eq!(carol["is_local"], false);
        assert_eq!(carol["server_name"], "remote.example");
        // The display name of the joined room wins over the later leave.
        assert_eq!(carol["displayname"], "Carol");
        assert_eq!(carol["joined_rooms"], 1);
        assert_eq!(carol["left_rooms"], 1);
        assert_eq!(carol["total_rooms"], 2);
        assert_eq!(body["users"][0]["is_local"], true);
        assert_eq!(body["users"][2]["banned_rooms"], 1);

        let (_, body) = request(
            &service,
            "GET",
            "v1/known_users?search_term=ku&local=false",
            None,
        )
        .await;
        assert_eq!(body["total"], 2);
        let (_, body) = request(
            &service,
            "GET",
            "v1/known_users?search_term=ku&local=true",
            None,
        )
        .await;
        assert_eq!(user_ids(&body), vec![alice.to_string()]);
        let (_, body) = request(
            &service,
            "GET",
            "v1/known_users?search_term=ku&server_name=other.example",
            None,
        )
        .await;
        assert_eq!(user_ids(&body), vec!["@kudave:other.example".to_owned()]);
        let (_, body) = request(
            &service,
            "GET",
            "v1/known_users?search_term=ku&order_by=total_rooms&dir=b&limit=1",
            None,
        )
        .await;
        assert_eq!(user_ids(&body), vec!["@kucarol:remote.example".to_owned()]);
        assert_eq!(body["next_token"], "1");

        let (status, body) = request(
            &service,
            "GET",
            "v1/known_users/@kucarol:remote.example",
            None,
        )
        .await;
        assert_eq!(status, Some(StatusCode::OK));
        assert_eq!(body["has_account"], false);
        assert_eq!(body["displayname"], "Carol");
        let memberships: Vec<&str> = body["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["membership"].as_str().unwrap())
            .collect();
        assert_eq!(memberships, vec!["leave", "join"]);

        let (_, body) = request(&service, "GET", &format!("v1/known_users/{alice}"), None).await;
        assert_eq!(body["has_account"], true);
        let (status, _) = request(
            &service,
            "GET",
            "v1/known_users/@kunobody:remote.example",
            None,
        )
        .await;
        assert_eq!(status, Some(StatusCode::NOT_FOUND));
    }
}
