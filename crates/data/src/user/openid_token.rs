use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

/// Look up the user and expiry recorded for an OpenID token.
pub async fn get_openid_token(token: &str) -> DataResult<Option<(OwnedUserId, UnixMillis)>> {
    user_openid_tokens::table
        .filter(user_openid_tokens::token.eq(token))
        .select((user_openid_tokens::user_id, user_openid_tokens::expires_at))
        .first::<(OwnedUserId, UnixMillis)>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Remove an OpenID token.
pub async fn delete_openid_token(token: &str) -> DataResult<()> {
    diesel::delete(user_openid_tokens::table.filter(user_openid_tokens::token.eq(token)))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
