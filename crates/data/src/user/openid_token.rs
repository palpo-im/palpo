use diesel::prelude::*;
use horme::{OptionalExtension as _, RunQueryDsl};

use crate::DataResult;
use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;

/// Look up the user and expiry recorded for an OpenID token.
pub async fn get_openid_token(token: &str) -> DataResult<Option<(OwnedUserId, UnixMillis)>> {
    user_openid_tokens::table
        .filter(user_openid_tokens::token.eq(token))
        .select((user_openid_tokens::user_id, user_openid_tokens::expires_at))
        .first::<(OwnedUserId, UnixMillis)>(&mut crate::db())
        .await
        .optional()
        .map_err(Into::into)
}

/// Remove an OpenID token.
pub async fn delete_openid_token(token: &str) -> DataResult<()> {
    diesel::delete(user_openid_tokens::table.filter(user_openid_tokens::token.eq(token)))
        .execute(&mut crate::db())
        .await?;
    Ok(())
}
