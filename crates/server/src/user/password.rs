use super::DbUser;
use crate::core::identifiers::*;
use crate::{AppResult, MatrixError, data};

pub fn ensure_account_usable(user: &DbUser) -> AppResult<()> {
    if user.deactivated_at.is_some() {
        return Err(MatrixError::user_deactivated("the user has been deactivated").into());
    }
    if user.locked_at.is_some() {
        return Err(MatrixError::user_locked("the user has been locked").into());
    }
    if user.suspended_at.is_some() {
        return Err(MatrixError::user_suspended("the user has been suspended").into());
    }
    Ok(())
}

pub async fn verify_password(user: &DbUser, password: &str) -> AppResult<()> {
    ensure_account_usable(user)?;

    let hash = crate::user::get_password_hash(&user.id)
        .await
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

pub async fn get_password_hash(user_id: &UserId) -> AppResult<String> {
    Ok(data::user::get_password_hash(user_id).await?)
}

/// Set/update password hash for a user
pub async fn set_password(user_id: &UserId, password: &str) -> AppResult<()> {
    let hash = crate::utils::hash_password(password)?;
    data::user::set_password_hash(user_id, &hash).await?;
    Ok(())
}
