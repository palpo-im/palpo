use std::collections::HashMap;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

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
    pub updated_at: UnixMillis,
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
    pub updated_at: UnixMillis,
}

impl DbPresence {
    /// Creates a PresenceEvent from available data.
    pub async fn to_presence_event(&self, user_id: &UserId) -> DataResult<PresenceEvent> {
        self.to_presence_event_at(user_id, UnixMillis::now()).await
    }

    /// Creates the wire event as observed at a fixed time.
    ///
    /// Federation retries use the row's persisted `updated_at`, so rebuilding an
    /// unacknowledged EDU cannot change `last_active_ago` and therefore its transaction ID.
    async fn to_presence_event_at(
        &self,
        user_id: &UserId,
        observed_at: UnixMillis,
    ) -> DataResult<PresenceEvent> {
        let state = self
            .state
            .as_deref()
            .map(PresenceState::from)
            .unwrap_or_default();
        let last_active_ago = if state == PresenceState::Online {
            None
        } else {
            self.last_active_at
                .map(|last_active_at| observed_at.0.saturating_sub(last_active_at.0))
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
    maybe_last_presence(user_id)
        .await?
        .ok_or_else(|| MatrixError::not_found("No presence data found for user").into())
}

/// Returns the user's current presence, or `None` when they have no presence row.
pub async fn maybe_last_presence(user_id: &UserId) -> DataResult<Option<PresenceEvent>> {
    let presence = user_presences::table
        .filter(user_presences::user_id.eq(user_id))
        .first::<DbPresence>(&mut connect().await?)
        .await
        .optional()?;
    if let Some(data) = presence {
        Ok(Some(data.to_presence_event(user_id).await?))
    } else {
        Ok(None)
    }
}

/// Sequence number of the user's current presence row, if one exists.
pub async fn presence_sn(user_id: &UserId) -> DataResult<Option<i64>> {
    user_presences::table
        .filter(user_presences::user_id.eq(user_id))
        .select(user_presences::occur_sn)
        .first::<i64>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Keyset pagination bounds startup memory and concurrent recalculations.
pub async fn presence_user_ids_after(after: Option<&UserId>) -> DataResult<Vec<OwnedUserId>> {
    let mut query = user_presences::table
        .select(user_presences::user_id)
        .into_boxed();
    if let Some(after) = after {
        query = query.filter(user_presences::user_id.gt(after));
    }
    query
        .order(user_presences::user_id)
        .limit(100)
        .load(&mut connect().await?)
        .await
        .map_err(Into::into)
}

// Serialize publication with cursor snapshots, including recovery transactions.
const PRESENCE_STREAM_LOCK: i64 = 0x50414c50524553;
pub async fn lock_presence_stream(conn: &mut AsyncPgConnection) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
        .bind::<diesel::sql_types::BigInt, _>(PRESENCE_STREAM_LOCK)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn curr_sn_after_presence_writes(
    device: Option<(&UserId, &DeviceId)>,
) -> DataResult<i64> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            diesel::sql_query("SELECT pg_advisory_xact_lock_shared($1)")
                .bind::<diesel::sql_types::BigInt, _>(PRESENCE_STREAM_LOCK)
                .execute(&mut *conn)
                .await?;
            if let Some((user, device)) = device {
                super::device::lock_inbox_stream(conn, user, device).await?;
            }
            Ok(
                diesel::dsl::sql::<diesel::sql_types::BigInt>(
                    "SELECT last_value FROM occur_sn_seq",
                )
                .get_result(conn)
                .await?,
            )
        })
        .await
}

/// Change only the position, preserving concurrently updated status and timestamps.
/// The caller holds the stream lock until its transaction commits.
pub async fn restamp_presence_with_conn(
    conn: &mut AsyncPgConnection,
    user: &UserId,
) -> DataResult<bool> {
    Ok(
        diesel::update(user_presences::table.filter(user_presences::user_id.eq(user)))
            .set(
                user_presences::occur_sn.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>(
                    "nextval('occur_sn_seq')",
                )),
            )
            .execute(conn)
            .await?
            > 0,
    )
}

/// Atomically replace presence and publish its new stream position.
pub async fn set_presence(mut db_presence: NewDbPresence, force: bool) -> DataResult<bool> {
    connect()
        .await?
        .transaction::<_, crate::DataError, _>(async |conn| {
            lock_presence_stream(conn).await?;
            let old_state = user_presences::table
                .filter(user_presences::user_id.eq(&db_presence.user_id))
                .select(user_presences::state)
                .first::<Option<String>>(conn)
                .await
                .optional()?
                .flatten();
            if old_state.as_ref() == db_presence.state.as_ref() && !force {
                return Ok(false);
            }
            let occur_sn =
                diesel::dsl::sql::<diesel::sql_types::BigInt>("SELECT nextval('occur_sn_seq')")
                    .get_result::<i64>(conn)
                    .await?;
            db_presence.occur_sn = Some(occur_sn);
            diesel::insert_into(user_presences::table)
                .values(&db_presence)
                .on_conflict(user_presences::user_id)
                .do_update()
                // Match replacement semantics, including clearing nullable fields.
                .set((
                    user_presences::stream_id.eq(db_presence.stream_id),
                    user_presences::state.eq(&db_presence.state),
                    user_presences::status_msg.eq(&db_presence.status_msg),
                    user_presences::last_active_at.eq(db_presence.last_active_at),
                    user_presences::last_federation_update_at.eq(db_presence.last_federation_update_at),
                    user_presences::last_user_sync_at.eq(db_presence.last_user_sync_at),
                    user_presences::currently_active.eq(db_presence.currently_active),
                    user_presences::occur_sn.eq(occur_sn),
                    user_presences::updated_at.eq(db_presence.updated_at),
                ))
                .execute(conn)
                .await?;
            Ok(true)
        })
        .await
}

/// Removes the presence record for the given user from the database.
pub async fn remove_presence(user_id: &UserId) -> DataResult<()> {
    diesel::delete(user_presences::table.filter(user_presences::user_id.eq(user_id)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Returns the most recent presence updates that happened after the event with id `since`.
pub async fn presences_since(
    since_sn: i64,
) -> DataResult<HashMap<OwnedUserId, (i64, PresenceEvent)>> {
    let presences = user_presences::table
        .filter(user_presences::occur_sn.ge(since_sn))
        .load::<DbPresence>(&mut connect().await?)
        .await?;
    let mut result = HashMap::new();
    for presence in presences {
        let event = presence.to_presence_event(&presence.user_id).await?;
        result.insert(presence.user_id, (presence.occur_sn, event));
    }
    Ok(result)
}

/// Returns current presence rows inside one inclusive, stable EDU selection window.
///
/// Ordering is part of the contract: federation can stop after a bounded number of
/// updates and safely resume at the last returned sequence number.
pub async fn presences_between(
    since_sn: i64,
    through_sn: i64,
) -> DataResult<Vec<(OwnedUserId, (i64, PresenceEvent))>> {
    let presences = user_presences::table
        .filter(user_presences::occur_sn.ge(since_sn))
        .filter(user_presences::occur_sn.le(through_sn))
        .order((
            user_presences::occur_sn.asc(),
            user_presences::user_id.asc(),
        ))
        .load::<DbPresence>(&mut connect().await?)
        .await?;
    let mut result = Vec::with_capacity(presences.len());
    for presence in presences {
        let event = presence
            .to_presence_event_at(&presence.user_id, presence.updated_at)
            .await?;
        result.push((presence.user_id, (presence.occur_sn, event)));
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
