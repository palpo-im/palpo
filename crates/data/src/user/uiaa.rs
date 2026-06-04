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
