use rand::seq::SliceRandom;
use serde_json::value::to_raw_value;

use crate::core::UnixMillis;
use crate::core::appservice::query::{QueryRoomAliasReqArgs, query_room_alias_request};
use crate::core::client::room::AliasResBody;
use crate::core::events::TimelineEventType;
use crate::core::federation::query::directory_request;
use crate::core::identifiers::*;
use crate::data::room::DbRoomAlias;
use crate::data::user::DbUser;
use crate::exts::*;
use crate::room::{StateEventType, timeline};
use crate::{AppError, AppResult, GetUrlOrigin, MatrixError, PduBuilder, config, data};

mod namespace;
mod remote;
use remote::remote_resolve;

#[inline]
pub async fn resolve(room: &RoomOrAliasId) -> AppResult<OwnedRoomId> {
    resolve_with_servers(room, None)
        .await
        .map(|(room_id, _)| room_id)
}

pub async fn resolve_with_servers(
    room: &RoomOrAliasId,
    servers: Option<Vec<OwnedServerName>>,
) -> AppResult<(OwnedRoomId, Vec<OwnedServerName>)> {
    if room.is_room_id() {
        let room_id: &RoomId = room.try_into().expect("valid RoomId");
        Ok((room_id.to_owned(), servers.unwrap_or_default()))
    } else {
        let alias: &RoomAliasId = room.try_into().expect("valid RoomAliasId");
        resolve_alias(alias, servers).await
    }
}

#[tracing::instrument(name = "resolve")]
pub async fn resolve_alias(
    room_alias: &RoomAliasId,
    servers: Option<Vec<OwnedServerName>>,
) -> AppResult<(OwnedRoomId, Vec<OwnedServerName>)> {
    let server_name = room_alias.server_name();
    let is_local_server = server_name.is_local();
    let servers_contains_local = || {
        let conf = crate::config::get();
        servers
            .as_ref()
            .is_some_and(|servers| servers.contains(&conf.server_name))
    };

    if !is_local_server && !servers_contains_local() {
        return remote_resolve(room_alias, servers.unwrap_or_default()).await;
    }

    let room_id = match resolve_local_alias(room_alias).await {
        Ok(r) => r,
        Err(_) => resolve_appservice_alias(room_alias).await?,
    };

    Ok((room_id, Vec::new()))
}

#[tracing::instrument(level = "debug")]
pub async fn resolve_local_alias(alias_id: &RoomAliasId) -> AppResult<OwnedRoomId> {
    data::room::get_alias_room_id(alias_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("Room alias not found.").into())
}

async fn resolve_appservice_alias(room_alias: &RoomAliasId) -> AppResult<OwnedRoomId> {
    for appservice in crate::appservice::all().await?.values() {
        if appservice.aliases.is_match(room_alias.as_str())
            && let Some(url) = &appservice.registration.url
        {
            let request = query_room_alias_request(
                url,
                QueryRoomAliasReqArgs {
                    room_alias: room_alias.to_owned(),
                },
            )?
            .into_inner();

            match crate::sending::send_appservice_request::<Option<()>>(
                appservice.registration.clone(),
                request,
            )
            .await
            {
                Ok(Some(_)) => {
                    // Appservice acknowledged the alias, try resolving locally now
                    match resolve_local_alias(room_alias).await {
                        Ok(room_id) => return Ok(room_id),
                        Err(e) => {
                            warn!(
                                "Appservice {} claimed alias {} but it wasn't created locally: {}",
                                appservice.registration.id, room_alias, e
                            );
                        }
                    }
                }
                Ok(None) => {
                    debug!(
                        "Appservice {} did not claim alias {}",
                        appservice.registration.id, room_alias
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to query appservice {} for alias {}: {}",
                        appservice.registration.id, room_alias, e
                    );
                }
            }
        }
    }

    Err(MatrixError::not_found("resolve appservice alias not found").into())
}

pub async fn local_aliases_for_room(room_id: &RoomId) -> AppResult<Vec<OwnedRoomAliasId>> {
    Ok(data::room::local_aliases_for_room(room_id).await?)
}
pub async fn all_local_aliases() -> AppResult<Vec<(OwnedRoomId, String)>> {
    let lists = data::room::all_local_aliases()
        .await?
        .into_iter()
        .map(|(room_id, alias_id)| (room_id, alias_id.alias().to_owned()))
        .collect::<Vec<_>>();
    Ok(lists)
}

pub async fn is_admin_room(room_id: &RoomId) -> bool {
    admin_room_id()
        .await
        .is_ok_and(|admin_room_id| admin_room_id == room_id)
}

pub async fn admin_room_id() -> AppResult<OwnedRoomId> {
    crate::room::resolve_local_alias(
        <&RoomAliasId>::try_from(format!("#admins:{}", config::get().server_name).as_str())
            .expect("#admins:server_name is a valid room alias"),
    )
    .await
}

/// The admin room alias decides which room is the admin console, so only the
/// server user or a current local admin may point it at a room.
pub async fn ensure_can_claim_alias(alias_id: &RoomAliasId, user_id: &UserId) -> AppResult<()> {
    if alias_id != config::admin_alias() || user_id == config::server_user_id() {
        return Ok(());
    }
    if user_id.server_name() == config::server_name()
        && data::user::is_admin(user_id).await.unwrap_or(false)
    {
        return Ok(());
    }
    Err(MatrixError::forbidden("This room alias is reserved.", None).into())
}

/// Try to create a new mapping; false is an actual unique-insert conflict.
pub async fn claim_alias(
    room_id: impl Into<OwnedRoomId>,
    alias_id: impl Into<OwnedRoomAliasId>,
    created_by: impl Into<OwnedUserId>,
) -> AppResult<bool> {
    let alias_id = alias_id.into();
    let room_id = room_id.into();
    let created_by = created_by.into();
    ensure_can_claim_alias(&alias_id, &created_by).await?;
    Ok(data::room::set_alias(DbRoomAlias {
        alias_id,
        room_id,
        created_by,
        created_at: UnixMillis::now(),
        generation: 0,
    })
    .await?)
}

pub fn alias_conflict(message: &'static str) -> AppError {
    let mut error = MatrixError::unknown(message);
    error.status_code = Some(salvo::http::StatusCode::CONFLICT);
    error.into()
}

pub async fn set_alias(
    room_id: impl Into<OwnedRoomId>,
    alias_id: impl Into<OwnedRoomAliasId>,
    created_by: impl Into<OwnedUserId>,
) -> AppResult<()> {
    if !claim_alias(room_id, alias_id, created_by).await? {
        return Err(alias_conflict("Room alias already exists"));
    }
    Ok(())
}

/// Room upgrades move an existing local mapping without taking ownership from
/// its creator or reclaiming an application-service namespace.
pub async fn move_alias(
    alias_id: &RoomAliasId,
    old_room: &RoomId,
    new_room: &RoomId,
    actor: &UserId,
) -> AppResult<()> {
    ensure_can_claim_alias(alias_id, actor).await?;
    if let Some(expected) = data::room::get_alias(alias_id).await? {
        if expected.room_id != old_room {
            return Ok(());
        }
        let mut replacement = expected.clone();
        replacement.room_id = new_room.to_owned();
        if !data::room::replace_alias(&expected, &replacement).await? {
            // A concurrent replacement is a new mapping, outside this upgrade.
            return Err(alias_conflict(
                "Room alias changed during room upgrade; retry",
            ));
        }
    }
    Ok(())
}

/// Explicit administrator force-overwrite. Ordinary directory PUT never calls it.
pub async fn force_set_alias(
    room_id: &RoomId,
    alias_id: &RoomAliasId,
    user: &DbUser,
) -> AppResult<()> {
    if !user.is_admin && user.id != config::server_user_id() {
        return Err(MatrixError::forbidden(
            "Alias force-overwrite requires a server administrator",
            None,
        )
        .into());
    }
    ensure_can_claim_alias(alias_id, &user.id).await?;
    let replacement = DbRoomAlias {
        alias_id: alias_id.to_owned(),
        room_id: room_id.to_owned(),
        created_by: user.id.clone(),
        created_at: UnixMillis::now(),
        generation: 0,
    };
    let changed = if let Some(expected) = data::room::get_alias(alias_id).await? {
        data::room::replace_alias(&expected, &replacement).await?
    } else {
        data::room::set_alias(replacement).await?
    };
    if !changed {
        return Err(alias_conflict(
            "Room alias changed during force-overwrite; retry",
        ));
    }
    Ok(())
}

pub async fn get_alias_response(room_alias: OwnedRoomAliasId) -> AppResult<AliasResBody> {
    if room_alias.server_name() != config::get().server_name {
        let request =
            directory_request(&room_alias.server_name().origin().await, &room_alias)?.into_inner();
        let mut body =
            crate::sending::send_federation_request(room_alias.server_name(), request, None)
                .await?
                .json::<AliasResBody>()
                .await?;

        body.servers.shuffle(&mut rand::rng());

        return Ok(body);
    }

    let mut room_id = None;
    match resolve_local_alias(&room_alias).await {
        Ok(r) => room_id = Some(r),
        Err(_) => {
            for appservice in crate::appservice::all().await?.values() {
                let url = appservice
                    .registration
                    .build_url(&format!("app/v1/rooms/{room_alias}"))?;
                if appservice.aliases.is_match(room_alias.as_str())
                    && matches!(
                        crate::sending::post(url).send::<Option<()>>().await,
                        Ok(Some(_opt_result))
                    )
                {
                    room_id = Some(resolve_local_alias(&room_alias).await.map_err(|_| {
                        AppError::public("Appservice lied to us. Room does not exist.")
                    })?);
                    break;
                }
            }
        }
    };

    let room_id = match room_id {
        Some(room_id) => room_id,
        None => return Err(MatrixError::not_found("Room with alias not found.").into()),
    };

    Ok(AliasResBody::new(
        room_id,
        vec![config::get().server_name.to_owned()],
    ))
}

#[tracing::instrument]
/// Vendor/server operations retain explicit administrative authority.
pub async fn remove_alias(alias_id: &RoomAliasId, user: &DbUser) -> AppResult<()> {
    remove_alias_inner(alias_id, user, None, false).await
}

/// Standard C-S requests retain the authenticated AS identity across aliases.
pub async fn remove_alias_as_client(
    alias_id: &RoomAliasId,
    user: &DbUser,
    appservice: Option<&crate::appservice::RegistrationInfo>,
) -> AppResult<()> {
    remove_alias_inner(alias_id, user, appservice, true).await
}

pub async fn check_client_alias_namespace(
    alias_id: &RoomAliasId,
    appservice: Option<&crate::appservice::RegistrationInfo>,
) -> AppResult<()> {
    namespace::check(alias_id, appservice, true)
        .await
        .map(|_| ())
}

async fn remove_alias_inner(
    alias_id: &RoomAliasId,
    user: &DbUser,
    appservice: Option<&crate::appservice::RegistrationInfo>,
    client: bool,
) -> AppResult<()> {
    let namespace_control = if client {
        namespace::check(alias_id, appservice, false).await?
    } else {
        false
    };
    let expected = data::room::get_alias(alias_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("Room alias not found."))?;
    let room_id = expected.room_id.clone();
    if user_can_remove_alias(&expected, user, namespace_control).await? {
        let state_lock = super::lock_state(&room_id).await;
        let current = data::room::get_alias(alias_id).await?;
        if current.as_ref().is_none_or(|row| {
            row.room_id != expected.room_id
                || row.created_by != expected.created_by
                || row.created_at != expected.created_at
                || row.generation != expected.generation
        }) {
            return Err(alias_conflict("Room alias changed during deletion; retry"));
        }
        let namespace_control = if client {
            namespace::check(alias_id, appservice, false).await?
        } else {
            false
        };
        if !user_can_remove_alias(&expected, user, namespace_control).await? {
            return Err(MatrixError::forbidden(
                "User is not permitted to remove this alias.",
                None,
            )
            .into());
        }
        // Directory mappings may legitimately point at rooms not stored here.
        // State cleanup is optional; only an existing canonical event needs its
        // room version. Namespace/creator/admin authority still permits removal.
        let canonical = if super::room_exists(&room_id).await? {
            match super::get_state(&room_id, &StateEventType::RoomCanonicalAlias, "", None).await {
                Ok(event) => Some(event),
                Err(error) if error.is_not_found() => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        if let Some(event) = canonical {
            let mut content: serde_json::Value = serde_json::from_str(event.content.get())?;
            if remove_advertised_alias(&mut content, alias_id.as_str()) {
                // Directory deletion is permitted independently of room state rights.
                // A retained power level does not authorize a departed member to send state.
                // Do not discard unrelated preferred aliases, or publish a fake empty event.
                let can_update = super::user::is_joined(&user.id, &room_id).await?
                    && super::get_power_levels(&room_id)
                        .await?
                        .user_can_send_state(&user.id, StateEventType::RoomCanonicalAlias);
                if can_update {
                    let room_version = crate::room::get_version(&room_id).await?;
                    timeline::build_and_append_pdu(
                        PduBuilder {
                            event_type: TimelineEventType::RoomCanonicalAlias,
                            content: to_raw_value(&content)?,
                            state_key: Some("".to_owned()),
                            ..Default::default()
                        },
                        &user.id,
                        &room_id,
                        &room_version,
                        &state_lock,
                    )
                    .await?;
                }
            }
        }
        if !data::room::remove_alias(&expected).await? {
            return Err(alias_conflict("Room alias changed during deletion; retry"));
        }
        Ok(())
    } else {
        Err(MatrixError::forbidden("User is not permitted to remove this alias.", None).into())
    }
}
fn remove_advertised_alias(content: &mut serde_json::Value, target: &str) -> bool {
    let mut changed = false;
    if content["alias"].as_str() == Some(target) {
        content
            .as_object_mut()
            .expect("state content object")
            .remove("alias");
        changed = true;
    }
    if let Some(aliases) = content
        .get_mut("alt_aliases")
        .and_then(serde_json::Value::as_array_mut)
    {
        let before = aliases.len();
        aliases.retain(|alias| alias.as_str() != Some(target));
        changed |= before != aliases.len();
    }
    changed
}

#[tracing::instrument]
async fn user_can_remove_alias(
    alias: &DbRoomAlias,
    user: &DbUser,
    namespace_control: bool,
) -> AppResult<bool> {
    let room_id = &alias.room_id;

    // The creator of an alias can remove it
    if namespace_control || alias.created_by == user.id
        // Server admins can remove any local alias
        || user.is_admin
        // Always allow the Palpo user to remove the alias, since there may not be an admin room
        || config::server_user_id()== user.id
    {
        Ok(true)
        // Checking whether the user is able to change canonical aliases of the room
    } else if let Ok(power_levels) = super::get_power_levels(room_id).await {
        Ok(power_levels.user_can_send_state(&user.id, StateEventType::RoomCanonicalAlias))
    // If there is no power levels event, only the room creator can change canonical aliases
    } else if let Ok(event) = super::get_state(room_id, &StateEventType::RoomCreate, "", None).await
    {
        Ok(event.sender == user.id)
    } else {
        error!("Room {} has no m.room.create event (VERY BAD)!", room_id);
        Err(AppError::public("Room has no m.room.create event"))
    }
}

#[cfg(test)]
mod advertised_alias_tests {
    use serde_json::json;

    use super::remove_advertised_alias;
    #[test]
    fn removal_preserves_other_aliases_and_custom_content() {
        let mut state = json!({"alias":"#main:example","alt_aliases":["#a:example","#b:example"],"org.example.custom":true});
        assert!(remove_advertised_alias(&mut state, "#a:example"));
        assert_eq!(
            state,
            json!({"alias":"#main:example","alt_aliases":["#b:example"],"org.example.custom":true})
        );
        assert!(!remove_advertised_alias(&mut state, "#missing:example"));
        assert!(remove_advertised_alias(&mut state, "#main:example"));
        assert_eq!(state["alt_aliases"], json!(["#b:example"]));
        assert!(state.get("alias").is_none());
    }
}
