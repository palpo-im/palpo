use diesel::prelude::*;

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

pub fn get_password_hash(user_id: &UserId) -> DataResult<String> {
    user_passwords::table
        .filter(user_passwords::user_id.eq(user_id))
        .order_by(user_passwords::id.desc())
        .select(user_passwords::hash)
        .first::<String>(&mut connect()?)
        .map_err(Into::into)
}
