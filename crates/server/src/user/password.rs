use diesel::prelude::*;

use super::DbUser;
use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::data::connect;
use crate::data::schema::*;
use crate::data::user::NewDbPassword;
use crate::{AppResult, MatrixError, data, utils};

pub fn verify_password(user: &DbUser, password: &str) -> AppResult<()> {
    if user.deactivated_at.is_some() {
        return Err(MatrixError::user_deactivated("The user has been deactivated").into());
    }
    let hash = data::user::get_password_hash(&user.id)
        .map_err(|_| MatrixError::unauthorized("Wrong username or password."))?;
    if hash.is_empty() {
        return Err(MatrixError::user_deactivated("The user has been deactivated").into());
    }

    let hash_matches = argon2::verify_encoded(&hash, password.as_bytes()).unwrap_or(false);

    if !hash_matches {
        Err(MatrixError::unauthorized("Wrong username or password.").into())
    } else {
        Ok(())
    }
}

pub fn set_password(user_id: &UserId, password: &str) -> AppResult<()> {
    if let Ok(hash) = utils::hash_password(password) {
        diesel::insert_into(user_passwords::table)
            .values(NewDbPassword {
                user_id: user_id.to_owned(),
                hash,
                created_at: UnixMillis::now(),
            })
            .execute(&mut connect()?)?;
        Ok(())
    } else {
        Err(MatrixError::invalid_param("Password does not meet the requirements.").into())
    }
}
