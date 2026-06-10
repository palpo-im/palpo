use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_refresh_tokens)]
pub struct DbRefreshToken {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub next_token_id: Option<i64>,
    pub expires_at: i64,
    pub ultimate_session_expires_at: i64,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_refresh_tokens)]
pub struct NewDbRefreshToken {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub next_token_id: Option<i64>,
    pub expires_at: i64,
    pub ultimate_session_expires_at: i64,
    pub created_at: UnixMillis,
}

impl NewDbRefreshToken {
    pub fn new(
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        token: String,
        expires_at: i64,
        ultimate_session_expires_at: i64,
    ) -> Self {
        Self {
            user_id,
            device_id,
            token,
            next_token_id: None,
            expires_at,
            ultimate_session_expires_at,
            created_at: UnixMillis::now(),
        }
    }
}

/// Return the expiry of a refresh token if it exists for this `(user, device)`.
pub async fn get_refresh_token_expires_at(
    user_id: &UserId,
    device_id: &DeviceId,
    token: &str,
) -> DataResult<Option<i64>> {
    user_refresh_tokens::table
        .filter(user_refresh_tokens::user_id.eq(user_id))
        .filter(user_refresh_tokens::device_id.eq(device_id))
        .filter(user_refresh_tokens::token.eq(token))
        .select(user_refresh_tokens::expires_at)
        .first::<i64>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}
