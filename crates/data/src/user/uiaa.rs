use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::client::uiaa::UiaaInfo;
use crate::core::identifiers::*;
use crate::core::serde::{CanonicalJsonValue, JsonValue};
use crate::schema::*;
use crate::{DataResult, connect};

/// Upsert (when `uiaa_info` is `Some`) or delete (when `None`) the UIAA session
/// row for a `(user, device, session)` triple.
pub async fn update_session(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    uiaa_info: Option<&UiaaInfo>,
) -> DataResult<()> {
    if let Some(uiaa_info) = uiaa_info {
        let uiaa_info = serde_json::to_value(uiaa_info)?;
        diesel::insert_into(user_uiaa_datas::table)
            .values((
                user_uiaa_datas::user_id.eq(user_id),
                user_uiaa_datas::device_id.eq(device_id),
                user_uiaa_datas::session.eq(session),
                user_uiaa_datas::uiaa_info.eq(&uiaa_info),
            ))
            .on_conflict((
                user_uiaa_datas::user_id,
                user_uiaa_datas::device_id,
                user_uiaa_datas::session,
            ))
            .do_update()
            .set(user_uiaa_datas::uiaa_info.eq(&uiaa_info))
            .execute(&mut connect().await?)
            .await?;
    } else {
        diesel::delete(
            user_uiaa_datas::table
                .filter(user_uiaa_datas::user_id.eq(user_id))
                .filter(user_uiaa_datas::device_id.eq(device_id))
                .filter(user_uiaa_datas::session.eq(session)),
        )
        .execute(&mut connect().await?)
        .await?;
    };
    Ok(())
}

/// Fetch the stored `UiaaInfo` for a `(user, device, session)` triple.
pub async fn get_session(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> DataResult<UiaaInfo> {
    let uiaa_info = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::uiaa_info)
        .first::<JsonValue>(&mut connect().await?)
        .await?;
    Ok(serde_json::from_value(uiaa_info)?)
}

/// Store the UIAA request body in the database for cross-instance access.
pub async fn set_uiaa_request(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
    request: &CanonicalJsonValue,
) -> DataResult<()> {
    let request_body = serde_json::to_value(request)?;
    diesel::update(
        user_uiaa_datas::table
            .filter(user_uiaa_datas::user_id.eq(user_id))
            .filter(user_uiaa_datas::device_id.eq(device_id))
            .filter(user_uiaa_datas::session.eq(session)),
    )
    .set(user_uiaa_datas::request_body.eq(Some(&request_body)))
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}

/// Get the UIAA request body from the database.
pub async fn get_uiaa_request(
    user_id: &UserId,
    device_id: &DeviceId,
    session: &str,
) -> DataResult<Option<CanonicalJsonValue>> {
    let request_body = user_uiaa_datas::table
        .filter(user_uiaa_datas::user_id.eq(user_id))
        .filter(user_uiaa_datas::device_id.eq(device_id))
        .filter(user_uiaa_datas::session.eq(session))
        .select(user_uiaa_datas::request_body)
        .first::<Option<JsonValue>>(&mut connect().await?)
        .await
        .optional()?
        .flatten();

    match request_body {
        Some(body) => Ok(Some(serde_json::from_value(body)?)),
        None => Ok(None),
    }
}

/// Store a registration challenge and its non-secret attempt binding together.
/// Completion uses the normal UIAA row deletion, including this metadata.
pub async fn create_registration_session(
    user_id: &UserId,
    device_id: &DeviceId,
    uiaa_info: &UiaaInfo,
    fingerprint: &str,
    expires_at: i64,
) -> DataResult<()> {
    let info = serde_json::to_value(uiaa_info)?;
    let binding = serde_json::json!({
        "palpo.registration": {"fingerprint": fingerprint, "expires_at": expires_at}
    });
    diesel::insert_into(user_uiaa_datas::table)
        .values((
            user_uiaa_datas::user_id.eq(user_id),
            user_uiaa_datas::device_id.eq(device_id),
            user_uiaa_datas::session.eq(uiaa_info.session.as_deref().expect("session is set")),
            user_uiaa_datas::uiaa_info.eq(&info),
            user_uiaa_datas::request_body.eq(Some(&binding)),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Find only the same registration attempt, pruning its expired challenges.
/// Other UIAA request bodies and unrelated registrations are not affected.
pub async fn registration_session(
    user_id: &UserId,
    device_id: &DeviceId,
    fingerprint: &str,
    now: i64,
) -> DataResult<Option<UiaaInfo>> {
    use diesel::sql_types::{BigInt, Json, Text};
    #[derive(QueryableByName)]
    struct StoredInfo {
        #[diesel(sql_type = Json)]
        uiaa_info: JsonValue,
    }
    let mut conn = connect().await?;
    // JSONB numeric comparison avoids casting arbitrary legacy request bodies.
    diesel::sql_query(
        "DELETE FROM user_uiaa_datas WHERE user_id = $1 AND device_id = $2
         AND request_body->'palpo.registration'->>'fingerprint' = $3
         AND request_body->'palpo.registration'->'expires_at' <= to_jsonb($4::bigint)",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(device_id.as_str())
    .bind::<Text, _>(fingerprint)
    .bind::<BigInt, _>(now)
    .execute(&mut conn)
    .await?;
    let stored = diesel::sql_query(
        "SELECT uiaa_info FROM user_uiaa_datas WHERE user_id = $1 AND device_id = $2
         AND request_body->'palpo.registration'->>'fingerprint' = $3
         AND jsonb_typeof(request_body->'palpo.registration'->'expires_at') = 'number'
         AND request_body->'palpo.registration'->'expires_at' > to_jsonb($4::bigint)
         ORDER BY id DESC LIMIT 1",
    )
    .bind::<Text, _>(user_id.as_str())
    .bind::<Text, _>(device_id.as_str())
    .bind::<Text, _>(fingerprint)
    .bind::<BigInt, _>(now)
    .get_result::<StoredInfo>(&mut conn)
    .await
    .optional()?;
    stored
        .map(|row| serde_json::from_value(row.uiaa_info).map_err(Into::into))
        .transpose()
}
