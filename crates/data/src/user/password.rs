use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Debug, Clone)]
#[diesel(table_name = user_passwords)]
pub struct DbPassword {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub hash: String,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Queryable, Debug, Clone)]
#[diesel(table_name = user_passwords)]
pub struct NewDbPassword {
    pub user_id: OwnedUserId,
    pub hash: String,
    pub created_at: UnixMillis,
}

/// Return the most recent password hash for a user.
pub async fn get_password_hash(user_id: &UserId) -> DataResult<String> {
    user_passwords::table
        .filter(user_passwords::user_id.eq(user_id))
        .order_by(user_passwords::id.desc())
        .select(user_passwords::hash)
        .first::<String>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Store a new password hash for a user and clear the guest flag.
pub async fn set_password_hash(user_id: &UserId, hash: &str) -> DataResult<()> {
    diesel::insert_into(user_passwords::table)
        .values(NewDbPassword {
            user_id: user_id.to_owned(),
            hash: hash.to_owned(),
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect().await?)
        .await?;
    diesel::update(users::table.find(user_id))
        .set(users::is_guest.eq(false))
        .execute(&mut connect().await?)
        .await?;
    super::access_token::invalidate_user(user_id);
    Ok(())
}
