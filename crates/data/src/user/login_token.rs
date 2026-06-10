use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

/// Create or refresh a short-lived `m.login.token` login token.
pub async fn upsert_login_token(user_id: &UserId, token: &str, expires_at: i64) -> DataResult<()> {
    diesel::insert_into(user_login_tokens::table)
        .values((
            user_login_tokens::user_id.eq(user_id),
            user_login_tokens::token.eq(token),
            user_login_tokens::expires_at.eq(expires_at),
        ))
        .on_conflict(user_login_tokens::token)
        .do_update()
        .set(user_login_tokens::expires_at.eq(expires_at))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Look up the user and expiry recorded for a login token.
pub async fn get_login_token(token: &str) -> DataResult<Option<(OwnedUserId, UnixMillis)>> {
    user_login_tokens::table
        .filter(user_login_tokens::token.eq(token))
        .select((user_login_tokens::user_id, user_login_tokens::expires_at))
        .first::<(OwnedUserId, UnixMillis)>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Remove a login token.
pub async fn delete_login_token(token: &str) -> DataResult<()> {
    diesel::delete(user_login_tokens::table.filter(user_login_tokens::token.eq(token)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
