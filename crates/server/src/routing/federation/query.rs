//! Endpoints to retrieve information from a homeserver about a resource.

use palpo_core::federation::query::ProfileReqArgs;
use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::federation::query::{ProfileResBody, RoomInfoResBody};
use crate::core::identifiers::*;
use crate::core::profile::ProfileFieldValue;
use crate::{
    AuthArgs, EmptyResult, IsRemoteOrLocal, JsonResult, MatrixError, config, data, json_ok,
};

#[cfg(feature = "unstable-msc4495")]
pub fn unstable_router() -> Router {
    Router::with_path("org.continuwuity.presence_v2.msc4495/query")
        .push(Router::with_path("presence_recipients").get(get_presence_recipients))
}

pub fn router() -> Router {
    Router::with_path("query")
        .push(Router::with_path("profile").get(get_profile))
        .push(Router::with_path("directory").get(get_directory))
        .push(Router::with_path("{query_type}").get(query_by_type))
}

/// #GET /_matrix/federation/v1/query/profile
/// Gets information on a profile.
#[endpoint]
async fn get_profile(_aa: AuthArgs, args: ProfileReqArgs) -> JsonResult<ProfileResBody> {
    if !config::get().federation.allow_inbound_profile_lookup {
        return Err(MatrixError::forbidden(
            "Profile lookup over federation is disabled on this homeserver",
            None,
        )
        .into());
    }

    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }

    let mut response = ProfileResBody::new();

    let Some(profile) = data::user::get_profile(&args.user_id, None).await? else {
        if data::user::active_local_user_exists(&args.user_id).await? {
            return json_ok(response);
        }
        return Err(MatrixError::not_found("Profile not found.").into());
    };
    let custom_fields = profile.fields.as_object().cloned().unwrap_or_default();

    match args.field.as_ref().map(|field| field.as_str()) {
        Some("displayname") => {
            if let Some(display_name) = profile.display_name {
                response.extend([ProfileFieldValue::DisplayName(display_name)]);
            }
        }
        Some("avatar_url") => {
            if let Some(avatar_url) = profile.avatar_url {
                response.extend([ProfileFieldValue::AvatarUrl(avatar_url)]);
            }
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
        }
        Some("xyz.amorgan.blurhash") => {
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
        }
        Some(field) => {
            if let Some(value) = custom_fields.get(field) {
                response.set(field, value.clone());
            }
        }
        None => {
            if let Some(display_name) = profile.display_name {
                response.extend([ProfileFieldValue::DisplayName(display_name)]);
            }
            if let Some(avatar_url) = profile.avatar_url {
                response.extend([ProfileFieldValue::AvatarUrl(avatar_url)]);
            }
            if let Some(blurhash) = profile.blurhash {
                response.set("xyz.amorgan.blurhash", blurhash.into());
            }
            response.extend(custom_fields);
        }
    }

    json_ok(response)
}

/// #GET /_matrix/federation/v1/query/directory
/// Resolve a room alias to a room id.
#[endpoint]
async fn get_directory(
    _aa: AuthArgs,
    room_alias: QueryParam<OwnedRoomAliasId, true>,
) -> JsonResult<RoomInfoResBody> {
    let room_id = crate::room::resolve_local_alias(&room_alias).await?;
    let mut servers = crate::room::lookup_servers(&room_id).await?;
    servers.insert(0, config::get().server_name.to_owned());
    servers.dedup();
    json_ok(RoomInfoResBody { room_id, servers })
}
#[endpoint]
async fn query_by_type(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::unrecognized("Unsupported federation query type.").into())
}

#[cfg(test)]
mod profile_tests {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use salvo::test::{ResponseExt, TestClient};
    use serde_json::{Value, json};

    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run this test alone"]
    async fn profile_lookup_routes_preserve_accounts_without_profile_rows() {
        crate::test_database::init();
        config::CONFIG
            .set(
                serde_json::from_value(json!({
                    "server_name":"profile.example", "db":{"url":"unused-test-config"},
                    "ip_range_denylist":[], "federation":{"allow_inbound_profile_lookup":true}
                }))
                .unwrap(),
            )
            .unwrap();
        for name in ["empty", "named", "deactivated"] {
            let id = UserId::parse(format!("@{name}:profile.example")).unwrap();
            data::user::create_user(&data::user::NewDbUser {
                id: id.clone(),
                ty: None,
                is_admin: false,
                is_guest: false,
                is_local: true,
                localpart: name.to_owned(),
                server_name: id.server_name().to_owned(),
                appservice_id: None,
                created_at: crate::core::UnixMillis::now(),
            })
            .await
            .unwrap();
        }
        let named = UserId::parse("@named:profile.example").unwrap();
        data::user::ensure_profile_exists(&named).await.unwrap();
        data::user::set_display_name(&named, "A named user")
            .await
            .unwrap();
        data::user::set_profile_field(&named, "org.example.note", json!("preserved"))
            .await
            .unwrap();
        let deactivated = UserId::parse("@deactivated:profile.example").unwrap();
        data::user::mark_deactivated(&deactivated).await.unwrap();

        let client = Service::new(crate::routing::root());
        // Exercise the federation query handler directly; authentication is covered
        // by the federation hoop tests and is not bypassed in production routing.
        let federation = Service::new(router());
        for name in ["empty", "missing", "deactivated", "named"] {
            let id = format!("@{name}:profile.example");
            let expected_status = if matches!(name, "empty" | "named") {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            };
            for path in [
                format!("/_matrix/client/v3/profile/{id}"),
                format!("/_matrix/client/r0/profile/{id}"),
            ] {
                let mut response = TestClient::get(format!("http://localhost{path}"))
                    .send(&client)
                    .await;
                assert_eq!(response.status_code, Some(expected_status), "{path}");
                let body = response.take_json::<Value>().await.unwrap();
                if name == "empty" {
                    assert_eq!(body, json!({}));
                }
                if name == "named" {
                    assert_eq!(body["displayname"], "A named user");
                    assert_eq!(body["org.example.note"], "preserved");
                }
                if expected_status == StatusCode::NOT_FOUND {
                    assert_eq!(body["errcode"], "M_NOT_FOUND");
                }
            }
            for field in [None, Some("displayname"), Some("avatar_url")] {
                let suffix = field
                    .map(|field| format!("&field={field}"))
                    .unwrap_or_default();
                let mut response = TestClient::get(format!(
                    "http://localhost/query/profile?user_id={id}{suffix}"
                ))
                .send(&federation)
                .await;
                assert_eq!(
                    response.status_code,
                    Some(expected_status),
                    "{name} {field:?}"
                );
                let body = response.take_json::<Value>().await.unwrap();
                if name == "empty" {
                    assert_eq!(body, json!({}));
                }
                if name == "named" && field != Some("avatar_url") {
                    assert_eq!(body["displayname"], "A named user");
                }
            }
        }
        assert_eq!(
            TestClient::get("http://localhost/query/profile?user_id=@remote:elsewhere.example")
                .send(&federation)
                .await
                .status_code,
            Some(StatusCode::BAD_REQUEST)
        );
        let empty = UserId::parse("@empty:profile.example").unwrap();
        assert!(
            data::user::get_profile(&empty, None)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            data::schema::user_profiles::table
                .count()
                .get_result::<i64>(&mut data::connect().await.unwrap())
                .await
                .unwrap(),
            1
        );
    }
}

/// #GET /_matrix/federation/unstable/org.continuwuity.presence_v2.msc4495/query/presence_recipients
/// Returns a local user's current presence recipient set for the asking server ([MSC4495]).
///
/// A server whose view of the set has fallen out of step -- a delta whose `prev_id` it does
/// not hold -- calls this to resynchronise. Only the asking server's own users are
/// returned; the set for another server is none of its business, and the proposal scopes
/// the answer that way for exactly that reason.
///
/// [MSC4495]: https://github.com/matrix-org/matrix-spec-proposals/pull/4495
#[cfg(feature = "unstable-msc4495")]
#[endpoint]
pub(super) async fn get_presence_recipients(
    _aa: AuthArgs,
    args: crate::core::federation::query::PresenceRecipientsReqArgs,
    depot: &mut Depot,
) -> JsonResult<crate::core::federation::query::PresenceRecipientsResBody> {
    use crate::DepotExt;
    use crate::user::presence::{recipients, sharing};

    let origin = depot.origin()?.clone();

    if args.user_id.server_name().is_remote() {
        return Err(MatrixError::invalid_param("User does not belong to this server.").into());
    }
    // Checked before any stream mutation: `advance_stream` inserts a durable row, and a
    // peer naming arbitrary nonexistent local IDs must not be able to grow that table.
    if !data::user::user_exists(&args.user_id).await? {
        return Err(MatrixError::not_found("No presence recipients for this server.").into());
    }

    // A policy delta can be selected while this snapshot is being computed. Fence the
    // confirmed write against the user's stream row; if another state reserved a newer
    // position first, recompute rather than overwriting its pending removal with stale
    // recipients.
    for _ in 0..4 {
        let snapshot: Vec<_> = sharing::recipients_of(&args.user_id)
            .await?
            .into_iter()
            .filter(|recipient| recipient.server_name() == origin)
            .collect();

        let stream_id = recipients::advance_stream(&args.user_id).await?;
        if snapshot.is_empty() {
            if recipients::forget_confirmed(&args.user_id, &origin, stream_id).await? {
                // The proposal answers 404 when there is no set for the asking server,
                // which keeps "shares with nobody here" distinguishable from "set is
                // momentarily empty".
                return Err(
                    MatrixError::not_found("No presence recipients for this server.").into(),
                );
            }
            continue;
        }

        if recipients::record_confirmed(
            &args.user_id,
            &origin,
            stream_id,
            &snapshot.iter().cloned().collect(),
        )
        .await?
        {
            return json_ok(
                crate::core::federation::query::PresenceRecipientsResBody::new(stream_id, snapshot),
            );
        }
    }

    Err(crate::AppError::internal(
        "presence recipient policy kept changing while building its snapshot",
    ))
}
