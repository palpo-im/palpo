//! Local MSC1763 content retention, with permanent expiry and preserved event graphs.
use diesel::prelude::*;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde_json::Value;

use crate::core::events::room::retention::RoomRetentionEventContent as Policy;
use crate::core::{EventId, OwnedEventId, OwnedRoomId, RoomId, UnixMillis};
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppError, AppResult, MatrixError, config, room};

pub const EVENT_TYPE: &str = "org.matrix.msc1763.retention";

pub fn enabled() -> bool {
    config::CONFIG
        .get()
        .is_some_and(|conf| conf.retention.enable)
}

#[derive(QueryableByName)]
struct Flag {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    value: bool,
}

#[derive(QueryableByName)]
struct Sanitized {
    #[diesel(sql_type = diesel::sql_types::Json)]
    value: Value,
}

pub async fn was_expired(event_id: &EventId) -> AppResult<bool> {
    if crate::data::DIESEL_POOL.get().is_none() {
        return Ok(false);
    }
    Ok(diesel::sql_query(
        "SELECT EXISTS(SELECT 1 FROM event_retention_expired WHERE event_id = $1) AS value",
    )
    .bind::<diesel::sql_types::Text, _>(event_id.as_str())
    .get_result::<Flag>(&mut connect().await?)
    .await?
    .value)
}

pub fn effective_policy(
    room_id: &RoomId,
) -> futures_util::future::BoxFuture<'_, AppResult<Policy>> {
    Box::pin(effective_policy_inner(room_id))
}

async fn effective_policy_inner(room_id: &RoomId) -> AppResult<Policy> {
    let conf = &config::get().retention;
    if let Some(policy) = conf.policies.get(room_id.as_str()) {
        return Ok(policy.clone());
    }
    let room_policy = if let Some(frame) = room::get_current_frame_id(room_id).await? {
        match room::state::get_state_event_id(frame, &EVENT_TYPE.into(), "").await {
            Ok(event_id) => {
                let json = event_datas::table
                    .find(event_id)
                    .select(event_datas::json_data)
                    .first::<Value>(&mut connect().await?)
                    .await?;
                // Invalid remote policy content must not compromise room operation.
                // Treat it as an explicit policy with no bounds, then apply server limits.
                Some(
                    serde_json::from_value(json.get("content").cloned().unwrap_or_default())
                        .unwrap_or_default(),
                )
            }
            Err(error) if error.is_not_found() => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    Ok(conf.effective(room_id, room_policy))
}

fn lifetime_elapsed(timestamp: u64, lifetime: u64, now: u64) -> bool {
    now.checked_sub(timestamp)
        .is_some_and(|age| age >= lifetime)
}

pub fn is_expired<'a>(
    event_id: &'a EventId,
    room_id: &'a RoomId,
    state_key: Option<&'a str>,
    timestamp: UnixMillis,
) -> futures_util::future::BoxFuture<'a, AppResult<bool>> {
    // Visibility checks also occur inside nested state/membership operations.
    Box::pin(is_expired_inner(event_id, room_id, state_key, timestamp))
}

async fn is_expired_inner(
    event_id: &EventId,
    room_id: &RoomId,
    state_key: Option<&str>,
    timestamp: UnixMillis,
) -> AppResult<bool> {
    if state_key.is_some() {
        return Ok(false);
    }
    if was_expired(event_id).await? {
        return Ok(true);
    }
    if !enabled() {
        return Ok(false);
    }
    let Some(lifetime) = effective_policy(room_id).await?.max_lifetime else {
        return Ok(false);
    };
    if !lifetime_elapsed(timestamp.get(), lifetime, UnixMillis::now().get()) {
        return Ok(false);
    }
    // Retain every live DAG tip, not just one branch's latest event.
    let terminal = diesel::select(diesel::dsl::exists(
        event_forward_extremities::table
            .filter(event_forward_extremities::room_id.eq(room_id))
            .filter(event_forward_extremities::event_id.eq(event_id)),
    ))
    .get_result::<bool>(&mut connect().await?)
    .await?;
    Ok(!terminal)
}

/// Strip expired payloads before any shared PDU loader returns them. The marker and
/// database trigger make expiry permanent across backfill, restarts and policy changes.
pub fn sanitize_json<'a>(
    event_id: &'a EventId,
    room_id: &'a RoomId,
    json: &'a mut Value,
) -> futures_util::future::BoxFuture<'a, AppResult<()>> {
    // PDU reads sit inside deeply nested room/federation futures. Keep the
    // retention transaction off their stack, including on ordinary state reads.
    Box::pin(sanitize_json_inner(event_id, room_id, json))
}

async fn sanitize_json_inner(
    event_id: &EventId,
    room_id: &RoomId,
    json: &mut Value,
) -> AppResult<()> {
    if json.get("state_key").is_some() {
        return Ok(());
    }
    let timestamp = json
        .get("origin_server_ts")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::internal("event has invalid timestamp"))?;
    if !is_expired(event_id, room_id, None, UnixMillis(timestamp)).await? {
        return Ok(());
    }
    if was_expired(event_id).await? {
        // Also scrub a snapshot read just before another node committed expiry.
        // Already-purged events need no repeated writes or index deletions.
        *json = diesel::sql_query("SELECT palpo_retention_scrub($1) AS value")
            .bind::<diesel::sql_types::Json, _>(&*json)
            .get_result::<Sanitized>(&mut connect().await?)
            .await?
            .value;
        return Ok(());
    }
    let mut conn = connect().await?;
    *json = conn.transaction::<_, AppError, _>(async |conn| {
        // Writers and the content guard lock the same event row. A re-fetched
        // federation event cannot overwrite a committed expiry marker.
        diesel::sql_query("SELECT id FROM events WHERE id = $1 FOR UPDATE")
            .bind::<diesel::sql_types::Text,_>(event_id.as_str()).execute(conn).await?;
        diesel::sql_query("INSERT INTO event_retention_expired(event_id, expired_at) VALUES ($1,$2) ON CONFLICT DO NOTHING")
            .bind::<diesel::sql_types::Text,_>(event_id.as_str()).bind::<diesel::sql_types::BigInt,_>(UnixMillis::now().get() as i64).execute(conn).await?;
        // The BEFORE trigger removes payload and unsigned bundles while preserving
        // signatures, hashes, prev/auth references, and necessary redaction references.
        diesel::sql_query("UPDATE event_datas SET json_data = palpo_retention_scrub(json_data) WHERE event_id = $1")
            .bind::<diesel::sql_types::Text,_>(event_id.as_str()).execute(conn).await?;
        diesel::delete(event_searches::table.filter(event_searches::event_id.eq(event_id))).execute(conn).await?;
        diesel::delete(event_relations::table.filter(event_relations::event_id.eq(event_id).or(event_relations::child_id.eq(event_id)))).execute(conn).await?;
        diesel::delete(event_stickies::table.filter(event_stickies::event_id.eq(event_id))).execute(conn).await?;
        diesel::delete(event_push_actions::table.filter(event_push_actions::event_id.eq(event_id))).execute(conn).await?;
        diesel::update(delayed_events::table.filter(delayed_events::event_id.eq(event_id)))
            .set(delayed_events::content.eq(serde_json::json!({}))).execute(conn).await?;
        event_datas::table.find(event_id).select(event_datas::json_data).first::<Value>(conn).await.map_err(Into::into)
    }).await?;
    Ok(())
}

pub fn validate_local(event_type: &str, state_key: Option<&str>, content: &str) -> AppResult<()> {
    if event_type != EVENT_TYPE {
        return Ok(());
    }
    if !enabled() {
        return Err(MatrixError::forbidden("Message retention is disabled", None).into());
    }
    if state_key != Some("") {
        return Err(MatrixError::invalid_param("Retention requires the empty state key").into());
    }
    serde_json::from_str::<Policy>(content)
        .map_err(|error| MatrixError::invalid_param(error.to_string()))?;
    Ok(())
}

pub async fn sweep() -> AppResult<()> {
    if !enabled() {
        return Ok(());
    }
    let mut cursor = 0i64;
    loop {
        let batch = events::table
            .filter(events::state_key.is_null())
            .filter(events::sn.gt(cursor))
            .order(events::sn.asc())
            .limit(200)
            .select((events::id, events::room_id, events::sn))
            .load::<(OwnedEventId, OwnedRoomId, i64)>(&mut connect().await?)
            .await?;
        if batch.is_empty() {
            break;
        }
        for (event_id, room_id, sn) in batch {
            cursor = sn;
            let json = event_datas::table
                .find(&event_id)
                .select(event_datas::json_data)
                .first::<Value>(&mut connect().await?)
                .await
                .optional()?;
            if let Some(mut json) = json
                && let Err(error) = sanitize_json(&event_id, &room_id, &mut json).await
            {
                tracing::error!(?error, %event_id, "message retention could not purge event; will retry");
            }
        }
        tokio::task::yield_now().await;
    }
    Ok(())
}

pub fn start() {
    tokio::spawn(async {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = sweep().await {
                tracing::error!(?error, "message retention sweep failed; will retry");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_expiry_boundaries_and_future_timestamps() {
        assert!(!lifetime_elapsed(1000, 100, 1099));
        assert!(lifetime_elapsed(1000, 100, 1100));
        assert!(lifetime_elapsed(1000, 0, 1000));
        assert!(!lifetime_elapsed(1000, 0, 999));
        assert!(!lifetime_elapsed(u64::MAX - 1, 100, u64::MAX));
    }
}

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_http_tests;
