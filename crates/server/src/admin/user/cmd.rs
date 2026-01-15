use crate::admin::{Context, get_room_info, parse_active_local_user_id, parse_local_user_id};
use crate::core::events::room::power_levels::RoomPowerLevelsEventContent;
use crate::core::events::room::redaction::RoomRedactionEventContent;
use crate::core::events::tag::{TagEventContent, TagInfo};
use crate::core::events::{RoomAccountDataEventType, StateEventType};
use crate::core::{OwnedEventId, OwnedRoomId, OwnedRoomOrAliasId, OwnedUserId};
use crate::room::timeline;
use crate::user::full_user_deactivate;
use crate::{
    AppError, AppResult, IsRemoteOrLocal, OptionalExtension, PduBuilder, config, data, membership,
    utils,
};

const AUTO_GEN_PASSWORD_LENGTH: usize = 25;
const BULK_JOIN_REASON: &str = "Bulk force joining this room as initiated by the server admin.";

pub(super) async fn list_users(ctx: &Context<'_>) -> AppResult<()> {
    let users: Vec<_> = crate::user::list_local_users()?;

    let mut plain_msg = format!("Found {} local user account(s):\n```\n", users.len());
    plain_msg += users
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .as_str();
    plain_msg += "\n```";

    ctx.write_str(&plain_msg).await
}

pub(super) async fn create_user(
    ctx: &Context<'_>,
    username: String,
    password: Option<String>,
) -> AppResult<()> {
    // Validate user id
    let user_id = parse_local_user_id(&username)?;
    let conf = config::get();

    if let Err(e) = user_id.validate_strict()
        && conf.emergency_password.is_none()
    {
        return Err(AppError::public(format!(
            "Username {user_id} contains disallowed characters or spaces: {e}"
        )));
    }

    if data::user::user_exists(&user_id)? {
        return Err(AppError::public(format!("User {user_id} already exists")));
    }

    let password = password.unwrap_or_else(|| utils::random_string(AUTO_GEN_PASSWORD_LENGTH));

    // Create user
    crate::user::create_user(&user_id, Some(password.as_str()))?;

    // Default to pretty displayname
    let display_name = user_id.localpart().to_owned();

    // If `new_user_displayname_suffix` is set, registration will push whatever
    // content is set to the user's display name with a space before it
    // if !conf.new_user_displayname_suffix.is_empty() {
    //     write!(
    //         displayname,
    //         " {}",
    //         conf.new_user_displayname_suffix
    //     )?;
    // }

    crate::data::user::set_display_name(&user_id, &display_name)?;

    // Initial account data
    crate::user::set_data(
        &user_id,
        None,
        &crate::core::events::GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(crate::core::events::push_rules::PushRulesEvent {
            content: crate::core::events::push_rules::PushRulesEventContent {
                global: crate::core::push::Ruleset::server_default(&user_id),
            },
        })?,
    )?;

    if !conf.auto_join_rooms.is_empty() {
        for room in &conf.auto_join_rooms {
            let Ok(room_id) = crate::room::alias::resolve(room).await else {
                error!(
                    %user_id,
                    "Failed to resolve room alias to room ID when attempting to auto join {room}, skipping"
                );
                continue;
            };

            if !crate::room::is_server_joined(config::server_name(), &room_id)? {
                warn!("Skipping room {room} to automatically join as we have never joined before.");
                continue;
            }

            if let Ok(room_server_name) = room.server_name() {
                let user = data::user::get_user(&user_id)?;
                match membership::join_room(
                    &user,
                    None,
                    &room_id,
                    Some("Automatically joining this room upon registration".to_owned()),
                    &[
                        config::server_name().to_owned(),
                        room_server_name.to_owned(),
                    ],
                    None,
                    None,
                    Default::default(),
                )
                .await
                {
                    Ok(_response) => {
                        info!("Automatically joined room {room} for user {user_id}");
                    }
                    Err(e) => {
                        // don't return this error so we don't fail registrations
                        error!("Failed to automatically join room {room} for user {user_id}: {e}");
                        crate::admin::send_text(&format!(
                            "Failed to automatically join room {room} for user {user_id}: \
								 {e}"
                        ))
                        .await?;
                    }
                }
            }
        }
    }

    // we dont add a device since we're not the user, just the creator

    // if this account creation is from the CLI / --execute, invite the first user
    // to admin room
    if let Ok(admin_room) = crate::room::get_admin_room() {
        if crate::room::joined_member_count(&admin_room).is_ok_and(|c| c == 1) {
            crate::user::make_user_admin(&user_id)?;
            warn!("Granting {user_id} admin privileges as the first user");
        }
    } else {
        debug!("create_user admin command called without an admin room being available");
    }

    ctx.write_str(&format!(
        "Created user with user_id: {user_id} and password: `{password}`"
    ))
    .await
}

pub(super) async fn deactivate(
    ctx: &Context<'_>,
    no_leave_rooms: bool,
    user_id: String,
) -> AppResult<()> {
    // Validate user id
    let user_id = parse_local_user_id(&user_id)?;

    // don't deactivate the server service account
    if user_id == config::server_user_id() {
        return Err(AppError::public(
            "Not allowed to deactivate the server service account.",
        ));
    }

    // TODO: admin
    // crate::user::deactivate_account(&user_id).await?;

    if !no_leave_rooms {
        crate::admin::send_text(&format!(
            "Making {user_id} leave all rooms after deactivation..."
        ))
        .await?;

        let all_joined_rooms: Vec<OwnedRoomId> = data::user::joined_rooms(&user_id)?;

        full_user_deactivate(&user_id, &all_joined_rooms).await?;

        membership::leave_all_rooms(&user_id).await?;
    }

    ctx.write_str(&format!("User {user_id} has been deactivated"))
        .await
}

pub(super) async fn reset_password(
    ctx: &Context<'_>,
    username: String,
    password: Option<String>,
) -> AppResult<()> {
    let user_id = parse_local_user_id(&username)?;

    if user_id == config::server_user_id() {
        return Err(AppError::public(
            "Not allowed to set the password for the server account. Please use the emergency password config option.",
        ));
    }

    let new_password = password.unwrap_or_else(|| utils::random_string(AUTO_GEN_PASSWORD_LENGTH));

    match crate::user::set_password(&user_id, new_password.as_str()) {
        Err(e) => {
            return Err(AppError::public(format!(
                "Couldn't reset the password for user {user_id}: {e}"
            )));
        }
        Ok(()) => write!(
            ctx,
            "Successfully reset the password for user {user_id}: `{new_password}`"
        ),
    }
    .await
}

pub(super) async fn deactivate_all(
    ctx: &Context<'_>,
    no_leave_rooms: bool,
    force: bool,
) -> AppResult<()> {
    if ctx.body.len() < 2
        || !ctx.body[0].trim().starts_with("```")
        || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    let usernames = ctx
        .body
        .to_vec()
        .drain(1..ctx.body.len().saturating_sub(1))
        .collect::<Vec<_>>();

    let mut user_ids: Vec<OwnedUserId> = Vec::with_capacity(usernames.len());
    let mut admins = Vec::new();

    for username in usernames {
        match parse_active_local_user_id(username).await {
            Err(e) => {
                crate::admin::send_text(&format!(
                    "{username} is not a valid username, skipping over: {e}"
                ))
                .await?;

                continue;
            }
            Ok(user_id) => {
                if crate::data::user::is_admin(&user_id)? && !force {
                    crate::admin::send_text(&format!(
                        "{username} is an admin and --force is not set, skipping over"
                    ))
                    .await?;

                    admins.push(username);
                    continue;
                }

                // don't deactivate the server service account
                if user_id == config::server_user_id() {
                    crate::admin::send_text(&format!(
                        "{username} is the server service account, skipping over"
                    ))
                    .await?;

                    continue;
                }

                user_ids.push(user_id.to_owned());
            }
        }
    }

    let mut deactivation_count: usize = 0;

    for user_id in user_ids {
        match crate::user::deactivate_account(&user_id).await {
            Err(e) => {
                let _ = crate::admin::send_text(&format!("failed deactivating user: {e}")).await;
            }
            Ok(()) => {
                deactivation_count = deactivation_count.saturating_add(1);
                if !no_leave_rooms {
                    info!("Forcing user {user_id} to leave all rooms apart of deactivate-all");
                    let all_joined_rooms = data::user::joined_rooms(&user_id)?;

                    full_user_deactivate(&user_id, &all_joined_rooms).await?;

                    membership::leave_all_rooms(&user_id).await?;
                }
            }
        }
    }

    if admins.is_empty() {
        write!(ctx, "Deactivated {deactivation_count} accounts.")
    } else {
        write!(
            ctx,
            "Deactivated {deactivation_count} accounts.\nSkipped admin accounts: {}. Use \
			 --force to deactivate admin accounts",
            admins.join(", ")
        )
    }
    .await
}

pub(super) async fn list_joined_rooms(ctx: &Context<'_>, user_id: String) -> AppResult<()> {
    // Validate user id
    let user_id = parse_local_user_id(&user_id)?;

    let mut rooms: Vec<_> = data::user::joined_rooms(&user_id)?
        .iter()
        .map(|room_id| get_room_info(room_id))
        .collect();

    if rooms.is_empty() {
        return Err(AppError::public("User is not in any rooms."));
    }

    rooms.sort_by_key(|r| r.joined_members);
    rooms.reverse();

    let body = rooms
        .iter()
        .map(|info| {
            format!(
                "{}\tMembers: {}\tName: {}",
                info.id, info.joined_members, info.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    ctx.write_str(&format!(
        "Rooms {user_id} Joined ({}):\n```\n{body}\n```",
        rooms.len(),
    ))
    .await
}

pub(super) async fn force_join_list_of_local_users(
    ctx: &Context<'_>,
    room_id: OwnedRoomOrAliasId,
    yes_i_want_to_do_this: bool,
) -> AppResult<()> {
    if ctx.body.len() < 2
        || !ctx.body[0].trim().starts_with("```")
        || ctx.body.last().unwrap_or(&"").trim() != "```"
    {
        return Err(AppError::public(
            "Expected code block in command body. Add --help for details.",
        ));
    }

    if !yes_i_want_to_do_this {
        return Err(AppError::public(
            "You must pass the --yes-i-want-to-do-this-flag to ensure you really want to force \
			 bulk join all specified local users.",
        ));
    }

    let Ok(admin_room) = crate::room::get_admin_room() else {
        return Err(AppError::public(
            "There is not an admin room to check for server admins.",
        ));
    };

    let (room_id, servers) = crate::room::alias::resolve_with_servers(&room_id, None).await?;

    if !crate::room::is_server_joined(config::server_name(), &room_id)? {
        return Err(AppError::public("We are not joined in this room."));
    }

    let server_admins: Vec<_> = crate::room::active_local_users_in_room(&admin_room)?;

    if !crate::room::joined_users(&room_id, None)?
        .iter()
        .any(|user_id| server_admins.contains(&user_id.to_owned()))
    {
        return Err(AppError::public(
            "There is not a single server admin in the room.",
        ));
    }

    let usernames = ctx
        .body
        .to_vec()
        .drain(1..ctx.body.len().saturating_sub(1))
        .collect::<Vec<_>>();

    let mut user_ids: Vec<OwnedUserId> = Vec::with_capacity(usernames.len());

    for username in usernames {
        match parse_active_local_user_id(username).await {
            Ok(user_id) => {
                // don't make the server service account join
                if user_id == config::server_user_id() {
                    crate::admin::send_text(&format!(
                        "{username} is the server service account, skipping over"
                    ))
                    .await?;

                    continue;
                }

                user_ids.push(user_id);
            }
            Err(e) => {
                crate::admin::send_text(&format!(
                    "{username} is not a valid username, skipping over: {e}"
                ))
                .await?;

                continue;
            }
        }
    }

    let mut failed_joins: usize = 0;
    let mut successful_joins: usize = 0;

    for user_id in user_ids {
        let user = data::user::get_user(&user_id)?;
        match membership::join_room(
            &user,
            None,
            &room_id,
            Some(String::from(BULK_JOIN_REASON)),
            &servers,
            None,
            None,
            Default::default(),
        )
        .await
        {
            Ok(_res) => {
                successful_joins = successful_joins.saturating_add(1);
            }
            Err(e) => {
                warn!("Failed force joining {user_id} to {room_id} during bulk join: {e}");
                failed_joins = failed_joins.saturating_add(1);
            }
        }
    }

    ctx.write_str(&format!(
        "{successful_joins} local users have been joined to {room_id}. {failed_joins} joins \
		 failed.",
    ))
    .await
}

pub(super) async fn force_join_all_local_users(
    ctx: &Context<'_>,
    room_id: OwnedRoomOrAliasId,
    yes_i_want_to_do_this: bool,
) -> AppResult<()> {
    if !yes_i_want_to_do_this {
        return Err(AppError::public(
            "you must pass the --yes-i-want-to-do-this-flag to ensure you really want to force \
			 bulk join all local users.",
        ));
    }

    let Ok(admin_room) = crate::room::get_admin_room() else {
        return Err(AppError::public(
            "There is not an admin room to check for server admins.",
        ));
    };

    let (room_id, servers) = crate::room::alias::resolve_with_servers(&room_id, None).await?;

    if !crate::room::is_server_joined(config::server_name(), &room_id)? {
        return Err(AppError::public("we are not joined in this room"));
    }

    let server_admins: Vec<_> = crate::room::active_local_users_in_room(&admin_room)?;

    if !crate::room::joined_users(&room_id, None)?
        .iter()
        .any(|user_id| server_admins.contains(&user_id.to_owned()))
    {
        return Err(AppError::public(
            "there is not a single server admin in the room.",
        ));
    }

    let mut failed_joins: usize = 0;
    let mut successful_joins: usize = 0;

    for user_id in &data::user::list_local_users()? {
        let user = data::user::get_user(user_id)?;
        match membership::join_room(
            &user,
            None,
            &room_id,
            Some(String::from(BULK_JOIN_REASON)),
            &servers,
            None,
            None,
            Default::default(),
        )
        .await
        {
            Ok(_res) => {
                successful_joins = successful_joins.saturating_add(1);
            }
            Err(e) => {
                warn!("Failed force joining {user_id} to {room_id} during bulk join: {e}");
                failed_joins = failed_joins.saturating_add(1);
            }
        }
    }

    ctx.write_str(&format!(
        "{successful_joins} local users have been joined to {room_id}. {failed_joins} joins \
		 failed.",
    ))
    .await
}

pub(super) async fn force_join_room(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomOrAliasId,
) -> AppResult<()> {
    let user_id = parse_local_user_id(&user_id)?;
    let (room_id, servers) = crate::room::alias::resolve_with_servers(&room_id, None).await?;

    assert!(user_id.is_local(), "parsed user_id must be a local user");
    let user = data::user::get_user(&user_id)?;
    membership::join_room(
        &user,
        None,
        &room_id,
        None,
        &servers,
        None,
        None,
        Default::default(),
    )
    .await?;

    ctx.write_str(&format!("{user_id} has been joined to {room_id}.",))
        .await
}

pub(super) async fn force_leave_room(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomOrAliasId,
) -> AppResult<()> {
    let user_id = parse_local_user_id(&user_id)?;
    let room_id = crate::room::alias::resolve(&room_id).await?;

    assert!(user_id.is_local(), "parsed user_id must be a local user");

    if !crate::room::user::is_joined(&user_id, &room_id)? {
        return Err(AppError::public("{user_id} is not joined in the room"));
    }

    membership::leave_room(&user_id, &room_id, None).await?;

    ctx.write_str(&format!("{user_id} has left {room_id}.",))
        .await
}

pub(super) async fn force_demote(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomOrAliasId,
) -> AppResult<()> {
    let user_id = parse_local_user_id(&user_id)?;
    let room_id = crate::room::alias::resolve(&room_id).await?;
    let room_version = crate::room::get_version(&room_id)?;
    let version_rule = crate::room::get_version_rules(&room_version)?;

    assert!(user_id.is_local(), "parsed user_id must be a local user");
    let state_lock = crate::room::lock_state(&room_id).await;
    let room_power_levels = crate::room::get_power_levels(&room_id).await.ok();

    let user_can_demote_self =
        room_power_levels.as_ref().is_some_and(|power_levels| {
            power_levels.user_can_change_user_power_level(&user_id, &user_id)
        }) || crate::room::get_state(&room_id, &StateEventType::RoomCreate, "", None)
            .is_ok_and(|event| event.sender == user_id);

    if !user_can_demote_self {
        return Err(AppError::public(
            "User is not allowed to modify their own power levels in the room.",
        ));
    }

    let mut power_levels_content: RoomPowerLevelsEventContent = room_power_levels
        .map(TryInto::try_into)
        .transpose()?
        .unwrap_or_else(|| RoomPowerLevelsEventContent::new(&version_rule.authorization));
    power_levels_content.users.remove(&user_id);

    let event = timeline::build_and_append_pdu(
        PduBuilder::state(String::new(), &power_levels_content),
        &user_id,
        &room_id,
        &room_version,
        &state_lock,
    )
    .await?;

    ctx.write_str(&format!(
        "User {user_id} demoted themselves to the room default power level in {room_id} - \
		 {}",
        event.event_id
    ))
    .await
}

pub(super) async fn make_user_admin(ctx: &Context<'_>, user_id: String) -> AppResult<()> {
    let user_id = parse_local_user_id(&user_id)?;
    assert!(user_id.is_local(), "Parsed user_id must be a local user");

    crate::user::make_user_admin(&user_id)?;

    ctx.write_str(&format!("{user_id} has been granted admin privileges.",))
        .await
}

pub(super) async fn put_room_tag(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomId,
    tag: String,
) -> AppResult<()> {
    let user_id = parse_active_local_user_id(&user_id).await?;
    let mut tags_event_content = data::user::get_data::<TagEventContent>(
        &user_id,
        Some(&room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    tags_event_content
        .tags
        .insert(tag.clone().into(), TagInfo::new());

    crate::user::set_data(
        &user_id,
        Some(room_id.clone()),
        &RoomAccountDataEventType::Tag.to_string(),
        serde_json::to_value(tags_event_content).expect("to json value always works"),
    )?;

    ctx.write_str(&format!(
        "Successfully updated room account data for {user_id} and room {room_id} with tag {tag}"
    ))
    .await
}

pub(super) async fn delete_room_tag(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomId,
    tag: String,
) -> AppResult<()> {
    let user_id = parse_active_local_user_id(&user_id).await?;
    let mut tags_event_content = data::user::get_data::<TagEventContent>(
        &user_id,
        Some(&room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    tags_event_content.tags.remove(&tag.clone().into());

    crate::user::set_data(
        &user_id,
        Some(room_id.clone()),
        &RoomAccountDataEventType::Tag.to_string(),
        serde_json::to_value(tags_event_content).expect("to json value always works"),
    )?;

    ctx.write_str(&format!(
        "Successfully updated room account data for {user_id} and room {room_id}, deleting room \
    	 tag {tag}"
    ))
    .await
}

pub(super) async fn get_room_tags(
    ctx: &Context<'_>,
    user_id: String,
    room_id: OwnedRoomId,
) -> AppResult<()> {
    let user_id = parse_active_local_user_id(&user_id).await?;
    let tags_event_content = data::user::get_data::<TagEventContent>(
        &user_id,
        Some(&room_id),
        &RoomAccountDataEventType::Tag.to_string(),
    )
    .unwrap_or_default();

    ctx.write_str(&format!("```\n{:#?}\n```", tags_event_content.tags))
        .await
}

pub(super) async fn redact_event(ctx: &Context<'_>, event_id: OwnedEventId) -> AppResult<()> {
    let Ok(Some(event)) = timeline::get_non_outlier_pdu(&event_id) else {
        return Err(AppError::public("event does not exist in our database"));
    };

    // TODO: check if the user has permission to redact this event
    // if event.is_redacted() {
    //     return Err(AppError::public("event is already redacted"));
    // }

    if !event.sender.is_local() {
        return Err(AppError::public("this command only works on local users"));
    }

    let reason = format!(
        "the administrator(s) of {} has redacted this user's message",
        config::server_name()
    );

    let redaction_event = {
        let state_lock = crate::room::lock_state(&event.room_id).await;

        timeline::build_and_append_pdu(
            PduBuilder {
                redacts: Some(event.event_id.clone()),
                ..PduBuilder::timeline(&RoomRedactionEventContent {
                    redacts: Some(event.event_id.clone()),
                    reason: Some(reason),
                })
            },
            &event.sender,
            &event.room_id,
            &crate::room::get_version(&event.room_id)?,
            &state_lock,
        )
        .await?
    };

    ctx.write_str(&format!(
        "Successfully redacted event. Redaction event ID: {}",
        redaction_event.event_id
    ))
    .await
}
