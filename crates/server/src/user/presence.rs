#[cfg(not(feature = "unstable-msc4495"))]
use crate::core::federation::transaction::Edu;
use crate::core::presence::PresenceState;
#[cfg(not(feature = "unstable-msc4495"))]
use crate::core::presence::{PresenceContent, PresenceUpdate};
use crate::core::{UnixMillis, UserId};
use crate::data::user::{NewDbPresence, last_presence};
#[cfg(not(feature = "unstable-msc4495"))]
use crate::sending;
use crate::{AppResult, config, data};

#[cfg(feature = "unstable-msc4495")]
pub mod recipients;
#[cfg(feature = "unstable-msc4495")]
pub mod sharing;

/// Resets the presence timeout, so the user will stay in their current presence state.
pub async fn ping_presence(user_id: &UserId, new_state: &PresenceState) -> AppResult<()> {
    if !config::get().presence.allow_local {
        return Ok(());
    }

    const REFRESH_TIMEOUT: u64 = 60 * 1000;

    let last_presence = last_presence(user_id).await;
    let state_changed = match last_presence {
        Err(_) => true,
        Ok(ref presence) => presence.content.presence != *new_state,
    };

    let last_last_active_ago = match last_presence {
        Err(_) => 0_u64,
        Ok(ref presence) => presence.content.last_active_ago.unwrap_or_default(),
    };

    if !state_changed && last_last_active_ago < REFRESH_TIMEOUT {
        return Ok(());
    }

    let _status_msg = match last_presence {
        Ok(presence) => presence.content.status_msg.clone(),
        Err(_) => Some(String::new()),
    };

    let currently_active = *new_state == PresenceState::Online;

    data::user::set_presence(
        NewDbPresence {
            user_id: user_id.to_owned(),
            stream_id: None,
            state: Some(new_state.to_string()),
            status_msg: None,
            last_active_at: Some(UnixMillis::now()),
            last_federation_update_at: None,
            last_user_sync_at: None,
            currently_active: Some(currently_active),
            occur_sn: None,
        },
        false,
    )
    .await?;
    #[cfg(feature = "unstable-msc4495")]
    recipients::wake_recipient_servers(user_id).await?;
    Ok(())
}

/// Adds a presence event which will be saved until a new event replaces it.
pub async fn set_presence(
    sender_id: &UserId,
    presence_state: Option<PresenceState>,
    status_msg: Option<String>,
    force: bool,
) -> AppResult<bool> {
    if !config::get().presence.allow_local {
        return Ok(false);
    }

    let Some(presence_state) = presence_state else {
        data::user::remove_presence(sender_id).await?;
        return Ok(false);
    };
    let db_presence = NewDbPresence {
        user_id: sender_id.to_owned(),
        stream_id: None,
        state: Some(presence_state.to_string()),
        status_msg: status_msg.clone(),
        last_active_at: None,
        last_federation_update_at: None,
        last_user_sync_at: None,
        currently_active: Some(presence_state == PresenceState::Online),
        occur_sn: None,
    };

    #[cfg_attr(feature = "unstable-msc4495", allow(unused_variables))]
    let state_changed = data::user::set_presence(db_presence, force).await?;
    // With selective presence the recipient scoping lives in the sending guard's EDU
    // selection, which runs off the presence row just written. Broadcasting here as well
    // would put an update with no `stream_id` on the wire, and a peer implementing MSC4495
    // reads that as a legacy sender and shows it to everyone -- exactly what an absent or
    // empty sharing policy is supposed to prevent. The destinations still have to be woken,
    // or a quiet room would never get the update at all.
    #[cfg(feature = "unstable-msc4495")]
    if state_changed {
        recipients::wake_recipient_servers(sender_id).await?;
    }
    #[cfg(not(feature = "unstable-msc4495"))]
    if state_changed {
        let edu = Edu::Presence(PresenceContent {
            push: vec![PresenceUpdate {
                user_id: sender_id.to_owned(),
                status_msg,
                last_active_ago: 0,
                currently_active: presence_state == PresenceState::Online,
                presence: presence_state,
                #[cfg(feature = "unstable-msc4495")]
                recipients: None,
                #[cfg(feature = "unstable-msc4495")]
                stream_id: None,
                #[cfg(feature = "unstable-msc4495")]
                prev_id: None,
            }],
        });

        let joined_rooms = data::user::joined_rooms(sender_id).await?;
        let remote_servers = data::room::joined_servers_for_rooms(&joined_rooms).await?;

        sending::send_edu_servers(remote_servers.into_iter(), &edu).await?;
    }

    Ok(state_changed)
}
