use diesel::prelude::*;

use super::DbUser;
use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::NewDbPassword;
use crate::{AppResult, MatrixError};

pub fn verify_password(user: &DbUser, password: &str) -> AppResult<()> {
    if user.deactivated_at.is_some() {
        return Err(MatrixError::user_deactivated("the user has been deactivated").into());
    }
    let hash = crate::user::get_password_hash(&user.id)
        .map_err(|_| MatrixError::unauthorized("wrong username or password."))?;
    if hash.is_empty() {
        return Err(MatrixError::user_deactivated("the user has been deactivated").into());
    }

    let hash_matches = argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

    if !hash_matches {
        Err(MatrixError::unauthorized("wrong username or password.").into())
    } else {
        Ok(())
    }
}

pub fn get_password_hash(user_id: &UserId) -> AppResult<String> {
    user_passwords::table
        .filter(user_passwords::user_id.eq(user_id))
        .order_by(user_passwords::id.desc())
        .select(user_passwords::hash)
        .first::<String>(&mut connect()?)
        .map_err(Into::into)
}

/// Set/update password hash for a user
pub fn set_password(user_id: &UserId, password: &str) -> AppResult<()> {
    let hash = crate::utils::hash_password(password)?;
    diesel::insert_into(user_passwords::table)
        .values(NewDbPassword {
            user_id: user_id.to_owned(),
            hash: hash.to_owned(),
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect()?)?;
    diesel::update(users::table.find(user_id))
        .set(users::is_guest.eq(false))
        .execute(&mut connect()?)?;
    Ok(())
}
