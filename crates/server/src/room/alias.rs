use diesel::prelude::*;
use rand::seq::SliceRandom;
use serde_json::value::to_raw_value;

use crate::core::UnixMillis;
use crate::core::appservice::query::{QueryRoomAliasReqArgs, query_room_alias_request};
use crate::core::client::room::AliasResBody;
use crate::core::events::TimelineEventType;
use crate::core::events::room::canonical_alias::RoomCanonicalAliasEventContent;
use crate::core::federation::query::directory_request;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::DbUser;
use crate::exts::*;
use crate::room::{StateEventType, timeline};
use crate::{AppError, AppResult, GetUrlOrigin, MatrixError, PduBuilder, config};

mod remote;
use remote::remote_resolve;

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = room_aliases, primary_key(alias_id))]
pub struct DbRoomAlias {
    pub alias_id: OwnedRoomAliasId,
    pub room_id: OwnedRoomId,
    pub created_by: OwnedUserId,
    pub created_at: UnixMillis,
}

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

    let room_id = match resolve_local_alias(room_alias) {
        Ok(r) => r,
        Err(_) => resolve_appservice_alias(room_alias).await?,
    };

    Ok((room_id, Vec::new()))
}

#[tracing::instrument(level = "debug")]
pub fn resolve_local_alias(alias_id: &RoomAliasId) -> AppResult<OwnedRoomId> {
    let room_id = room_aliases::table
        .filter(room_aliases::alias_id.eq(alias_id))
        .select(room_aliases::room_id)
        .first::<String>(&mut connect()?)?;

    RoomId::parse(room_id).map_err(|_| AppError::public("Room ID is invalid."))
}

async fn resolve_appservice_alias(room_alias: &RoomAliasId) -> AppResult<OwnedRoomId> {
    for appservice in crate::appservice::all()?.values() {
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
                    match resolve_local_alias(room_alias) {
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

pub fn local_aliases_for_room(room_id: &RoomId) -> AppResult<Vec<OwnedRoomAliasId>> {
    room_aliases::table
        .filter(room_aliases::room_id.eq(room_id))
        .select(room_aliases::alias_id)
        .load::<OwnedRoomAliasId>(&mut connect()?)
        .map_err(Into::into)
}
pub fn all_local_aliases() -> AppResult<Vec<(OwnedRoomId, String)>> {
    let lists = room_aliases::table
        .select((room_aliases::room_id, room_aliases::alias_id))
        .load::<(OwnedRoomId, OwnedRoomAliasId)>(&mut connect()?)?
        .into_iter()
        .map(|(room_id, alias_id)| (room_id, alias_id.alias().to_owned()))
        .collect::<Vec<_>>();
    Ok(lists)
}

pub fn is_admin_room(room_id: &RoomId) -> bool {
    admin_room_id().is_ok_and(|admin_room_id| admin_room_id == room_id)
}

pub fn admin_room_id() -> AppResult<OwnedRoomId> {
    crate::room::resolve_local_alias(
        <&RoomAliasId>::try_from(format!("#admins:{}", &config::get().server_name).as_str())
            .expect("#admins:server_name is a valid room alias"),
    )
}

pub fn set_alias(
    room_id: impl Into<OwnedRoomId>,
    alias_id: impl Into<OwnedRoomAliasId>,
    created_by: impl Into<OwnedUserId>,
) -> AppResult<()> {
    let alias_id = alias_id.into();
    let room_id = room_id.into();

    diesel::insert_into(room_aliases::table)
        .values(DbRoomAlias {
            alias_id,
            room_id,
            created_by: created_by.into(),
            created_at: UnixMillis::now(),
        })
        .on_conflict_do_nothing()
        .execute(&mut connect()?)
        .map(|_| ())
        .map_err(Into::into)
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

        body.servers.shuffle(&mut rand::thread_rng());

        return Ok(body);
    }

    let mut room_id = None;
    match resolve_local_alias(&room_alias) {
        Ok(r) => room_id = Some(r),
        Err(_) => {
            for appservice in crate::appservice::all()?.values() {
                let url = appservice
                    .registration
                    .build_url(&format!("app/v1/rooms/{room_alias}"))?;
                if appservice.aliases.is_match(room_alias.as_str())
                    && matches!(
                        crate::sending::post(url).send::<Option<()>>().await,
                        Ok(Some(_opt_result))
                    )
                {
                    room_id = Some(resolve_local_alias(&room_alias).map_err(|_| {
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
pub async fn remove_alias(alias_id: &RoomAliasId, user: &DbUser) -> AppResult<()> {
    let room_id = resolve_local_alias(alias_id)?;
    let room_version = crate::room::get_version(&room_id)?;
    if user_can_remove_alias(alias_id, user).await? {
        let state_alias = super::get_canonical_alias(&room_id);

        if state_alias.is_ok() {
            timeline::build_and_append_pdu(
                PduBuilder {
                    event_type: TimelineEventType::RoomCanonicalAlias,
                    content: to_raw_value(&RoomCanonicalAliasEventContent {
                        alias: None,
                        alt_aliases: vec![], // TODO
                    })
                    .expect("We checked that alias earlier, it must be fine"),
                    state_key: Some("".to_owned()),
                    ..Default::default()
                },
                &user.id,
                &room_id,
                &room_version,
                &super::lock_state(&room_id).await,
            )
            .await
            .ok();
        }
        diesel::delete(room_aliases::table.filter(room_aliases::alias_id.eq(alias_id)))
            .execute(&mut connect()?)?;

        Ok(())
    } else {
        Err(MatrixError::forbidden("User is not permitted to remove this alias.", None).into())
    }
}
#[tracing::instrument]
async fn user_can_remove_alias(alias_id: &RoomAliasId, user: &DbUser) -> AppResult<bool> {
    let room_id = resolve_local_alias(alias_id)?;

    let alias = room_aliases::table
        .find(alias_id)
        .first::<DbRoomAlias>(&mut connect()?)?;

    // The creator of an alias can remove it
    if alias.created_by == user.id
        // Server admins can remove any local alias
        || user.is_admin
        // Always allow the Palpo user to remove the alias, since there may not be an admin room
        || config::server_user_id()== user.id
    {
        Ok(true)
        // Checking whether the user is able to change canonical aliases of the room
    } else if let Ok(power_levels) = super::get_power_levels(&room_id).await {
        Ok(power_levels.user_can_send_state(&user.id, StateEventType::RoomCanonicalAlias))
    // If there is no power levels event, only the room creator can change canonical aliases
    } else if let Ok(event) = super::get_state(&room_id, &StateEventType::RoomCreate, "", None) {
        Ok(event.sender == user.id)
    } else {
        error!("Room {} has no m.room.create event (VERY BAD)!", room_id);
        Err(AppError::public("Room has no m.room.create event"))
    }
}
