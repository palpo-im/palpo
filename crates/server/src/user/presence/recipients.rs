//! The recipient-set stream behind selective presence ([MSC4495]).
//!
//! Remote servers learn about a user's recipient set as deltas, not as a whole set on every
//! presence update, so producing a correct delta needs the set that was last sent to each
//! destination. That state lives in the database rather than in memory: a server that
//! forgot what it had sent would either resend everything or -- much worse -- never retract
//! a recipient the user has since denied.
//!
//! [MSC4495]: https://github.com/matrix-org/matrix-spec-proposals/pull/4495

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::core::identifiers::*;
use crate::core::presence::PresenceRecipientListUpdates;
use crate::core::{Seqnum, UnixMillis};
use crate::data::connect;
use crate::data::schema::*;
use crate::exts::IsRemoteOrLocal;
use crate::{AppError, AppResult};

/// What one destination server must be told about a user's recipient set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientDelta {
    /// The position the destination's view moves to.
    pub stream_id: Seqnum,
    /// The position the destination's view is expected to be at, absent if we have never
    /// told this destination anything.
    pub prev_id: Option<Seqnum>,
    /// Recipients added and removed since `prev_id`.
    pub updates: PresenceRecipientListUpdates,
}

/// Computes the add/delete lists between two views of a recipient set.
pub fn diff(
    previous: &BTreeSet<OwnedUserId>,
    current: &BTreeSet<OwnedUserId>,
) -> PresenceRecipientListUpdates {
    PresenceRecipientListUpdates::new(
        current.difference(previous).cloned().collect(),
        previous.difference(current).cloned().collect(),
    )
}

/// The user's current recipient-set stream position, or 0 if they have never had one.
pub async fn stream_id(user_id: &UserId) -> AppResult<Seqnum> {
    Ok(presence_recipient_streams::table
        .find(user_id)
        .select(presence_recipient_streams::stream_id)
        .first::<Seqnum>(&mut connect().await?)
        .await
        .optional()?
        .unwrap_or(0))
}

/// Moves the user's recipient-set stream on and returns the new position.
///
/// Positions come from the same sequence that orders everything else, so they are
/// monotonic across restarts and never repeat -- a remote server can always tell an update
/// it missed from one it has already applied.
pub async fn advance_stream(user_id: &UserId) -> AppResult<Seqnum> {
    let stream_id = crate::data::next_sn().await?;
    diesel::insert_into(presence_recipient_streams::table)
        .values((
            presence_recipient_streams::user_id.eq(user_id),
            presence_recipient_streams::stream_id.eq(stream_id),
        ))
        .on_conflict(presence_recipient_streams::user_id)
        .do_update()
        .set(presence_recipient_streams::stream_id.eq(stream_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(stream_id)
}

/// What a destination has been told about a user's recipient set.
#[derive(Debug, Clone, Default)]
pub struct SentState {
    /// The last acknowledged position and set.
    pub confirmed: Option<(Seqnum, BTreeSet<OwnedUserId>)>,
    /// A delta that has been put on the wire but not acknowledged yet.
    pub pending: Option<(Seqnum, BTreeSet<OwnedUserId>)>,
}

/// Reads what `server_id` has been told about `user_id`'s recipient set.
pub async fn sent_state(user_id: &UserId, server_id: &ServerName) -> AppResult<SentState> {
    let Some((stream_id, recipients, pending_stream_id, pending_recipients, pending_edu_sn)) =
        presence_recipient_sets::table
            .find((user_id, server_id))
            .select((
                presence_recipient_sets::stream_id,
                presence_recipient_sets::recipients,
                presence_recipient_sets::pending_stream_id,
                presence_recipient_sets::pending_recipients,
                presence_recipient_sets::pending_edu_sn,
            ))
            .first::<(
                Seqnum,
                serde_json::Value,
                Option<Seqnum>,
                Option<serde_json::Value>,
                Option<Seqnum>,
            )>(&mut connect().await?)
            .await
            .optional()?
    else {
        return Ok(SentState::default());
    };

    Ok(SentState {
        confirmed: Some((
            stream_id,
            serde_json::from_value(recipients).unwrap_or_default(),
        )),
        pending: pending_stream_id
            .zip(pending_recipients)
            .zip(pending_edu_sn)
            .map(|((stream_id, recipients), _)| {
                (
                    stream_id,
                    serde_json::from_value(recipients).unwrap_or_default(),
                )
            }),
    })
}

/// Records a delta that has been put on the wire but not acknowledged.
///
/// It is deliberately not written into the confirmed set yet: if the transaction carrying
/// it fails, the next pass has to be able to compute the same delta again. Writing it
/// straight through would lose a removal -- the next pass would find nothing left to
/// remove while the destination still held the recipient.
pub async fn record_pending(
    user_id: &UserId,
    server_id: &ServerName,
    stream_id: Seqnum,
    recipients: &BTreeSet<OwnedUserId>,
    edu_sn: Seqnum,
) -> AppResult<()> {
    let recipients = serde_json::to_value(recipients)?;
    diesel::insert_into(presence_recipient_sets::table)
        .values((
            presence_recipient_sets::user_id.eq(user_id),
            presence_recipient_sets::server_id.eq(server_id),
            // A row that has never been confirmed starts from "told nothing", which is
            // what the absent-confirmed case means to `delta_for`.
            presence_recipient_sets::stream_id.eq(0),
            presence_recipient_sets::recipients.eq(serde_json::json!([])),
            presence_recipient_sets::pending_stream_id.eq(stream_id),
            presence_recipient_sets::pending_recipients.eq(&recipients),
            presence_recipient_sets::pending_edu_sn.eq(edu_sn),
        ))
        .on_conflict((
            presence_recipient_sets::user_id,
            presence_recipient_sets::server_id,
        ))
        .do_update()
        .set((
            presence_recipient_sets::pending_stream_id.eq(stream_id),
            presence_recipient_sets::pending_recipients.eq(&recipients),
            presence_recipient_sets::pending_edu_sn.eq(edu_sn),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Discards an unconfirmed selection before rebuilding the destination's next batch.
/// Confirmed state is left untouched, so every rebuilt delta is still based on what the
/// peer is known to hold.
pub async fn clear_pending(server_id: &ServerName) -> AppResult<()> {
    diesel::update(
        presence_recipient_sets::table.filter(presence_recipient_sets::server_id.eq(server_id)),
    )
    .set((
        presence_recipient_sets::pending_stream_id.eq(None::<Seqnum>),
        presence_recipient_sets::pending_recipients.eq(None::<serde_json::Value>),
        presence_recipient_sets::pending_edu_sn.eq(None::<Seqnum>),
    ))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Advances the EDU cursor and promotes only the presence batch this transaction carried.
///
/// Both changes commit atomically. If the process stops before this transaction commits,
/// the active outgoing row and pending delta remain available for startup recovery. If it
/// stops afterwards, the cursor proves that the dynamic EDU already landed. Passing no
/// `presence_edu_sn` deliberately leaves pending presence untouched when the transaction
/// contained only receipts or device-list EDUs.
pub async fn confirm_sent(
    server_id: &ServerName,
    edu_sn: Seqnum,
    presence_edu_sn: Option<Seqnum>,
) -> AppResult<()> {
    connect()
        .await?
        .transaction::<_, AppError, _>(async |conn| {
            if let Some(presence_edu_sn) = presence_edu_sn {
                diesel::update(
                    presence_recipient_sets::table
                        .filter(presence_recipient_sets::server_id.eq(server_id))
                        .filter(presence_recipient_sets::pending_edu_sn.eq(presence_edu_sn)),
                )
                .set((
                    presence_recipient_sets::stream_id.eq(diesel::dsl::sql::<
                        diesel::sql_types::BigInt,
                    >(
                        "COALESCE(pending_stream_id, stream_id)",
                    )),
                    presence_recipient_sets::recipients.eq(diesel::dsl::sql::<
                        diesel::sql_types::Jsonb,
                    >(
                        "COALESCE(pending_recipients, recipients)",
                    )),
                    presence_recipient_sets::pending_stream_id.eq(None::<Seqnum>),
                    presence_recipient_sets::pending_recipients.eq(None::<serde_json::Value>),
                    presence_recipient_sets::pending_edu_sn.eq(None::<Seqnum>),
                ))
                .execute(conn)
                .await?;

                diesel::delete(
                    presence_recipient_sets::table
                        .filter(presence_recipient_sets::server_id.eq(server_id))
                        .filter(presence_recipient_sets::pending_stream_id.is_null())
                        .filter(presence_recipient_sets::recipients.eq(serde_json::json!([]))),
                )
                .execute(conn)
                .await?;
            }

            let now = UnixMillis::now().get() as i64;
            diesel::insert_into(outgoing_edu_cursors::table)
                .values((
                    outgoing_edu_cursors::server_id.eq(server_id),
                    outgoing_edu_cursors::edu_sn.eq(edu_sn),
                    outgoing_edu_cursors::updated_at.eq(now),
                ))
                .on_conflict(outgoing_edu_cursors::server_id)
                .do_update()
                .set((
                    outgoing_edu_cursors::edu_sn.eq(diesel::dsl::sql::<diesel::sql_types::BigInt>(
                        "GREATEST(outgoing_edu_cursors.edu_sn, excluded.edu_sn)",
                    )),
                    outgoing_edu_cursors::updated_at.eq(now),
                ))
                .execute(conn)
                .await?;
            Ok(())
        })
        .await
}

/// A remote user's recipient set as we currently understand it.
pub async fn remote_set(user_id: &UserId) -> AppResult<Option<(Seqnum, BTreeSet<OwnedUserId>)>> {
    let Some((stream_id, recipients)) = remote_presence_recipients::table
        .find(user_id)
        .select((
            remote_presence_recipients::stream_id,
            remote_presence_recipients::recipients,
        ))
        .first::<(Seqnum, serde_json::Value)>(&mut connect().await?)
        .await
        .optional()?
    else {
        return Ok(None);
    };

    Ok(Some((
        stream_id,
        serde_json::from_value(recipients).unwrap_or_default(),
    )))
}

/// Replaces our view of a remote user's recipient set.
pub async fn store_remote_set(
    user_id: &UserId,
    stream_id: Seqnum,
    recipients: &BTreeSet<OwnedUserId>,
) -> AppResult<()> {
    let recipients = serde_json::to_value(recipients)?;
    diesel::insert_into(remote_presence_recipients::table)
        .values((
            remote_presence_recipients::user_id.eq(user_id),
            remote_presence_recipients::stream_id.eq(stream_id),
            remote_presence_recipients::recipients.eq(&recipients),
        ))
        .on_conflict(remote_presence_recipients::user_id)
        .do_update()
        .set((
            remote_presence_recipients::stream_id.eq(stream_id),
            remote_presence_recipients::recipients.eq(&recipients),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Removes selective state when a remote origin sends a legacy presence update.
pub async fn clear_remote_set(user_id: &UserId) -> AppResult<()> {
    diesel::delete(remote_presence_recipients::table.find(user_id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Records a set a destination is known to hold, bypassing the pending step.
///
/// Used by the recovery endpoint: once we have handed a destination a snapshot in a
/// response it has received, that *is* its state, so there is nothing to confirm later.
pub async fn record_confirmed(
    user_id: &UserId,
    server_id: &ServerName,
    stream_id: Seqnum,
    recipients: &BTreeSet<OwnedUserId>,
) -> AppResult<()> {
    let recipients = serde_json::to_value(recipients)?;
    diesel::insert_into(presence_recipient_sets::table)
        .values((
            presence_recipient_sets::user_id.eq(user_id),
            presence_recipient_sets::server_id.eq(server_id),
            presence_recipient_sets::stream_id.eq(stream_id),
            presence_recipient_sets::recipients.eq(&recipients),
            presence_recipient_sets::pending_stream_id.eq(None::<Seqnum>),
            presence_recipient_sets::pending_recipients.eq(None::<serde_json::Value>),
            presence_recipient_sets::pending_edu_sn.eq(None::<Seqnum>),
        ))
        .on_conflict((
            presence_recipient_sets::user_id,
            presence_recipient_sets::server_id,
        ))
        .do_update()
        .set((
            presence_recipient_sets::stream_id.eq(stream_id),
            presence_recipient_sets::recipients.eq(&recipients),
            presence_recipient_sets::pending_stream_id.eq(None::<Seqnum>),
            presence_recipient_sets::pending_recipients.eq(None::<serde_json::Value>),
            presence_recipient_sets::pending_edu_sn.eq(None::<Seqnum>),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Wakes every destination that either needs the current set or was told an older set.
pub async fn wake_recipient_servers(user_id: &UserId) -> AppResult<()> {
    let mut servers: BTreeSet<OwnedServerName> = presence_recipient_sets::table
        .filter(presence_recipient_sets::user_id.eq(user_id))
        .select(presence_recipient_sets::server_id)
        .load(&mut connect().await?)
        .await?
        .into_iter()
        .collect();
    servers.extend(
        super::sharing::recipients_of(user_id)
            .await?
            .into_iter()
            .map(|recipient| recipient.server_name().to_owned()),
    );
    let initial_edu_cursor = crate::data::user::presence_sn(user_id)
        .await?
        .map(|sn| sn.saturating_sub(1));
    crate::sending::wake_servers(servers.into_iter(), initial_edu_cursor).await
}

/// Schedules a fresh recipient delta for a user whose sharing inputs changed.
///
/// Deltas ride along with presence updates, which are selected from the presence stream, so
/// a change that does not touch presence -- editing `m.presence.sharing`, most obviously --
/// would otherwise not reach anyone until the user's presence next moved. Re-stamping the
/// presence row puts the user back in the selection window without inventing a state
/// transition they did not make.
pub async fn mark_recipients_changed(user_id: &UserId) -> AppResult<()> {
    // One recipient-list change has one global position, regardless of how many
    // destination servers later select its per-server delta.
    advance_stream(user_id).await?;

    let Some(presence) = crate::data::user::maybe_last_presence(user_id).await? else {
        // No presence to re-send; the next transition will carry the new set.
        return Ok(());
    };
    let content = presence.content;

    crate::data::user::set_presence(
        crate::data::user::NewDbPresence {
            user_id: user_id.to_owned(),
            stream_id: None,
            state: Some(content.presence.to_string()),
            status_msg: content.status_msg,
            last_active_at: content.last_active_ago.map(|ago| {
                crate::core::UnixMillis(crate::core::UnixMillis::now().0.saturating_sub(ago))
            }),
            last_federation_update_at: None,
            last_user_sync_at: None,
            currently_active: content.currently_active,
            occur_sn: None,
        },
        true,
    )
    .await?;

    wake_recipient_servers(user_id).await?;
    Ok(())
}

/// Users currently being recalculated; `true` means another pass was requested meanwhile.
static RECALCULATING: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<OwnedUserId, bool>>,
> = std::sync::LazyLock::new(Default::default);

/// Recalculates one local user's recipients in the background, collapsing bursts without
/// losing a change that arrives while an earlier pass is still running.
pub fn schedule_recipients_changed(user_id: &UserId) {
    if !user_id.is_local() {
        return;
    }
    {
        let mut recalculating = RECALCULATING
            .lock()
            .expect("recipient recalculation map is not poisoned");
        if let Some(dirty) = recalculating.get_mut(user_id) {
            *dirty = true;
            return;
        }
        recalculating.insert(user_id.to_owned(), false);
    }

    let user_id = user_id.to_owned();
    tokio::spawn(async move {
        loop {
            if let Err(e) = mark_recipients_changed(&user_id).await {
                warn!(%user_id, error = %e, "failed to refresh presence recipients; retrying");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                continue;
            }
            let mut recalculating = RECALCULATING
                .lock()
                .expect("recipient recalculation map is not poisoned");
            match recalculating.get_mut(&user_id) {
                Some(dirty) if *dirty => *dirty = false,
                _ => {
                    recalculating.remove(&user_id);
                    break;
                }
            }
        }
    });
}

/// Recalculates every local member affected by a room membership or sharing-hint change.
static RECALCULATING_ROOMS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<OwnedRoomId, bool>>,
> = std::sync::LazyLock::new(Default::default);

pub fn schedule_room_recipients_changed(room_id: &RoomId) {
    {
        let mut recalculating = RECALCULATING_ROOMS
            .lock()
            .expect("room recipient recalculation map is not poisoned");
        if let Some(dirty) = recalculating.get_mut(room_id) {
            *dirty = true;
            return;
        }
        recalculating.insert(room_id.to_owned(), false);
    }

    let room_id = room_id.to_owned();
    tokio::spawn(async move {
        loop {
            match crate::room::user::local_users(&room_id).await {
                Ok(users) => {
                    for user_id in users {
                        schedule_recipients_changed(&user_id);
                    }
                }
                Err(e) => {
                    warn!(%room_id, error = %e, "failed to list presence-sharing users; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    continue;
                }
            }

            let mut recalculating = RECALCULATING_ROOMS
                .lock()
                .expect("room recipient recalculation map is not poisoned");
            match recalculating.get_mut(&room_id) {
                Some(dirty) if *dirty => *dirty = false,
                _ => {
                    recalculating.remove(&room_id);
                    break;
                }
            }
        }
    });
}

/// Users whose recipient snapshot is currently being fetched.
///
/// A peer can name any of its own users with a `prev_id` we do not hold, so the number of
/// resyncs it can provoke is bounded only by the number of users it has. Collapsing
/// concurrent requests for the same user keeps that from turning into an outbound request
/// per inbound EDU.
static RESYNCING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<OwnedUserId>>> =
    std::sync::LazyLock::new(Default::default);

/// Fetches a recipient snapshot in the background.
///
/// Deliberately not awaited by the caller: this runs while handling an inbound federation
/// transaction, which carries up to a hundred EDUs, and a peer whose snapshot endpoint
/// stalls would otherwise hold that transaction open for the timeout multiplied by the
/// number of EDUs it chose to send. The set stays empty until the fetch lands, which is
/// what the proposal asks for.
pub fn schedule_resync(origin: &ServerName, user_id: &UserId) {
    {
        let mut resyncing = RESYNCING.lock().expect("resync set is not poisoned");
        if !resyncing.insert(user_id.to_owned()) {
            return;
        }
    }

    let origin = origin.to_owned();
    let user_id = user_id.to_owned();
    tokio::spawn(async move {
        if let Err(e) = fetch_remote_set(&origin, &user_id).await {
            warn!(%user_id, %origin, error = %e, "failed to re-fetch presence recipients");
        }
        RESYNCING
            .lock()
            .expect("resync set is not poisoned")
            .remove(&user_id);
    });
}

/// Asks the origin server for a snapshot of a user's recipient set.
///
/// Used after a delta that does not fit our view. The proposal has the set treated as empty
/// until this answers, which is what `track_presence_recipients` leaves in place.
pub async fn fetch_remote_set(origin: &ServerName, user_id: &UserId) -> AppResult<()> {
    use crate::core::federation::query::{PresenceRecipientsReqArgs, presence_recipients_request};
    use crate::exts::*;

    let request = presence_recipients_request(
        &origin.origin().await,
        PresenceRecipientsReqArgs::new(user_id.to_owned()),
    )?
    .into_inner();

    let body = crate::sending::send_federation_request(origin, request, Some(30))
        .await?
        .json::<crate::core::federation::query::PresenceRecipientsResBody>()
        .await?;

    // The origin answers with its own users' view; anything else would be the origin
    // claiming recipients on servers it does not speak for.
    let recipients: BTreeSet<OwnedUserId> = body
        .recipients
        .into_iter()
        .filter(|recipient| recipient.server_name() == crate::config::server_name())
        .collect();

    store_remote_set(user_id, body.stream_id, &recipients).await
}

/// How an incoming `m.presence` update should be interpreted ([MSC4495] inbound rules).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    /// The sender does not implement selective presence. Fall back to showing the update
    /// to every local user who shares a room with them.
    Legacy,
    /// The sender's view is the one we already hold; show the update to the stored set.
    Unchanged,
    /// Apply this delta to the stored set and show the update to the result.
    Apply {
        stream_id: Seqnum,
        updates: PresenceRecipientListUpdates,
    },
    /// Our view is not the one the delta was built against. Ask the origin for a snapshot
    /// and, until it answers, treat the set as empty rather than guessing wider.
    Resync,
}

/// Classifies an incoming update against our stored view of the sender's recipient set.
pub fn classify(
    known: Option<Seqnum>,
    stream_id: Option<Seqnum>,
    prev_id: Option<Seqnum>,
    updates: &PresenceRecipientListUpdates,
) -> Inbound {
    let Some(stream_id) = stream_id else {
        return Inbound::Legacy;
    };

    match (updates.is_empty(), prev_id) {
        // No delta and no prev_id: the sender is telling us its set is unchanged. That is
        // only meaningful if our view is at the position it names.
        (true, None) => {
            if known == Some(stream_id) {
                Inbound::Unchanged
            } else {
                Inbound::Resync
            }
        }
        // A delta with no prev_id initialises the set, but only when we hold nothing;
        // otherwise we would silently drop whatever we had.
        (false, None) => {
            if known.is_none() {
                Inbound::Apply {
                    stream_id,
                    updates: updates.clone(),
                }
            } else {
                Inbound::Resync
            }
        }
        // A delta with a prev_id applies only on top of exactly that position.
        (_, Some(prev_id)) => {
            if known == Some(prev_id) {
                Inbound::Apply {
                    stream_id,
                    updates: updates.clone(),
                }
            } else {
                Inbound::Resync
            }
        }
    }
}

/// The position stored for a set we know is out of date.
///
/// Real positions come from the sequence and are positive, so this matches no `prev_id` a
/// peer can send. That is the point: until a snapshot arrives, every further update from
/// that user classifies as [`Inbound::Resync`] and retries the fetch, instead of a later
/// delta appearing to apply cleanly on top of a set we know is wrong.
pub const UNKNOWN_STREAM_ID: Seqnum = Seqnum::MIN;

/// Applies a delta to a recipient set.
pub fn apply(set: &mut BTreeSet<OwnedUserId>, updates: &PresenceRecipientListUpdates) {
    for user_id in &updates.delete {
        set.remove(user_id);
    }
    set.extend(updates.add.iter().cloned());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Inbound, UNKNOWN_STREAM_ID, apply, classify, diff};
    use crate::core::identifiers::*;
    use crate::core::owned_user_id;
    use crate::core::presence::PresenceRecipientListUpdates;

    fn set(users: &[&str]) -> BTreeSet<OwnedUserId> {
        users
            .iter()
            .map(|user| UserId::parse(*user).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_delta_names_both_directions() {
        let updates = diff(
            &set(&["@a:example.org", "@b:example.org"]),
            &set(&["@b:example.org", "@c:example.org"]),
        );

        assert_eq!(updates.add, vec![owned_user_id!("@c:example.org")]);
        assert_eq!(updates.delete, vec![owned_user_id!("@a:example.org")]);
    }

    #[test]
    fn applying_a_delta_round_trips_the_set() {
        let previous = set(&["@a:example.org", "@b:example.org"]);
        let current = set(&["@b:example.org", "@c:example.org"]);

        let mut rebuilt = previous.clone();
        apply(&mut rebuilt, &diff(&previous, &current));

        assert_eq!(rebuilt, current);
    }

    #[test]
    fn an_update_without_a_stream_id_is_a_legacy_sender() {
        assert_eq!(
            classify(
                Some(7),
                None,
                None,
                &PresenceRecipientListUpdates::default()
            ),
            Inbound::Legacy
        );
    }

    #[test]
    fn an_empty_update_confirms_the_position_we_hold() {
        let empty = PresenceRecipientListUpdates::default();

        assert_eq!(classify(Some(7), Some(7), None, &empty), Inbound::Unchanged);
        // A position we do not hold means we missed something in between.
        assert_eq!(classify(Some(6), Some(7), None, &empty), Inbound::Resync);
        assert_eq!(classify(None, Some(7), None, &empty), Inbound::Resync);
    }

    #[test]
    fn a_delta_without_a_prev_id_only_initialises() {
        let updates =
            PresenceRecipientListUpdates::new(vec![owned_user_id!("@a:example.org")], Vec::new());

        assert_eq!(
            classify(None, Some(7), None, &updates),
            Inbound::Apply {
                stream_id: 7,
                updates: updates.clone()
            }
        );
        // We already hold a set; taking this as a fresh start would drop recipients we
        // were told about earlier.
        assert_eq!(classify(Some(3), Some(7), None, &updates), Inbound::Resync);
    }

    #[test]
    fn a_set_marked_unknown_keeps_forcing_a_resync() {
        let updates =
            PresenceRecipientListUpdates::new(vec![owned_user_id!("@a:example.org")], Vec::new());

        // Whatever the sender claims, a set we have marked unknown must not accept a
        // delta: it would look applied while still missing everything we never fetched.
        assert_eq!(
            classify(Some(UNKNOWN_STREAM_ID), Some(7), Some(3), &updates),
            Inbound::Resync
        );
        assert_eq!(
            classify(
                Some(UNKNOWN_STREAM_ID),
                Some(7),
                None,
                &PresenceRecipientListUpdates::default()
            ),
            Inbound::Resync
        );
    }

    #[test]
    fn a_delta_applies_only_on_top_of_its_own_prev_id() {
        let updates =
            PresenceRecipientListUpdates::new(vec![owned_user_id!("@a:example.org")], Vec::new());

        assert_eq!(
            classify(Some(3), Some(7), Some(3), &updates),
            Inbound::Apply {
                stream_id: 7,
                updates: updates.clone()
            }
        );
        assert_eq!(
            classify(Some(4), Some(7), Some(3), &updates),
            Inbound::Resync
        );
        assert_eq!(classify(None, Some(7), Some(3), &updates), Inbound::Resync);
    }
}
