use std::collections::HashMap;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::events::presence::{PresenceEvent, PresenceEventContent};
use crate::core::identifiers::*;
use crate::core::presence::PresenceState;
use crate::core::{MatrixError, UnixMillis};
use crate::schema::*;
use crate::{DataResult, connect};

/// Represents data required to be kept in order to implement the presence specification.
#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_presences)]
pub struct DbPresence {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub stream_id: Option<i64>,
    pub state: Option<String>,
    pub status_msg: Option<String>,
    pub last_active_at: Option<UnixMillis>,
    pub last_federation_update_at: Option<UnixMillis>,
    pub last_user_sync_at: Option<UnixMillis>,
    pub currently_active: Option<bool>,
    pub occur_sn: i64,
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = user_presences)]
pub struct NewDbPresence {
    pub user_id: OwnedUserId,
    pub stream_id: Option<i64>,
    pub state: Option<String>,
    pub status_msg: Option<String>,
    pub last_active_at: Option<UnixMillis>,
    pub last_federation_update_at: Option<UnixMillis>,
    pub last_user_sync_at: Option<UnixMillis>,
    pub currently_active: Option<bool>,
    pub occur_sn: Option<i64>,
}

impl DbPresence {
    /// Creates a PresenceEvent from available data.
    pub async fn to_presence_event(&self, user_id: &UserId) -> DataResult<PresenceEvent> {
        let now = UnixMillis::now();
        let state = self
            .state
            .as_deref()
            .map(PresenceState::from)
            .unwrap_or_default();
        let last_active_ago = if state == PresenceState::Online {
            None
        } else {
            self.last_active_at
                .map(|last_active_at| now.0.saturating_sub(last_active_at.0))
        };

        let profile = crate::user::get_profile(user_id, None).await?;
        Ok(PresenceEvent {
            sender: user_id.to_owned(),
            content: PresenceEventContent {
                presence: state,
                status_msg: self.status_msg.clone(),
                currently_active: self.currently_active,
                last_active_ago,
                display_name: profile.as_ref().and_then(|p| p.display_name.clone()),
                avatar_url: profile.as_ref().and_then(|p| p.avatar_url.clone()),
            },
        })
    }
}

pub async fn last_presence(user_id: &UserId) -> DataResult<PresenceEvent> {
    let presence = user_presences::table
        .filter(user_presences::user_id.eq(user_id))
        .first::<DbPresence>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(data) = presence {
        Ok(data.to_presence_event(user_id).await?)
    } else {
        Err(MatrixError::not_found("No presence data found for user").into())
    }
}

/// Adds a presence event which will be saved until a new event replaces it.
pub async fn set_presence(db_presence: NewDbPresence, force: bool) -> DataResult<bool> {
    let mut state_changed = false;
    let sender_id = &db_presence.user_id;
    let old_state = user_presences::table
        .filter(user_presences::user_id.eq(sender_id))
        .select(user_presences::state)
        .first::<Option<String>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();

    if old_state.as_ref() != db_presence.state.as_ref() || force {
        diesel::delete(user_presences::table.filter(user_presences::user_id.eq(sender_id)))
            .execute(&mut connect().await?)
            .await?;
        diesel::insert_into(user_presences::table)
            .values(&db_presence)
            .on_conflict(user_presences::user_id)
            .do_update()
            .set(&db_presence)
            .execute(&mut connect().await?)
            .await?;
        state_changed = true;
    }
    Ok(state_changed)
}

/// Removes the presence record for the given user from the database.
pub async fn remove_presence(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_presences::table.filter(user_presences::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Returns the most recent presence updates that happened after the event with id `since`.
pub async fn presences_since(since_sn: i64) -> DataResult<HashMap<OwnedUserId, PresenceEvent>> {
    let presences = user_presences::table
        .filter(user_presences::occur_sn.ge(since_sn))
        .load::<DbPresence>(&mut connect().await?)
        .await?;
    let mut result = HashMap::new();
    for presence in presences {
        let event = presence.to_presence_event(&presence.user_id).await?;
        result.insert(presence.user_id, event);
    }
    Ok(result)
}

// Unset online/unavailable presence to offline on startup
pub async fn unset_all_presences() -> DataResult<()> {
    diesel::delete(user_presences::table)
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
