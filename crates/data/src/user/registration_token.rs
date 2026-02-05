use diesel::prelude::*;

use crate::core::UnixMillis;
use crate::schema::*;
use crate::{DataResult, connect};

/// Database model for registration tokens
#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_registration_tokens)]
pub struct DbRegistrationToken {
    pub id: i64,
    pub token: String,
    pub uses_allowed: Option<i64>,
    pub pending: i64,
    pub completed: i64,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_registration_tokens)]
pub struct NewDbRegistrationToken {
    pub token: String,
    pub uses_allowed: Option<i64>,
    pub pending: i64,
    pub completed: i64,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

impl NewDbRegistrationToken {
    pub fn new(token: String, uses_allowed: Option<i64>, expires_at: Option<i64>) -> Self {
        Self {
            token,
            uses_allowed,
            pending: 0,
            completed: 0,
            expires_at,
            created_at: UnixMillis::now().get() as i64,
        }
    }
}

/// Info returned by the admin API
#[derive(Debug, Clone)]
pub struct RegistrationTokenInfo {
    pub token: String,
    pub uses_allowed: Option<i64>,
    pub pending: i64,
    pub completed: i64,
    pub expiry_time: Option<i64>,
}

impl From<DbRegistrationToken> for RegistrationTokenInfo {
    fn from(db: DbRegistrationToken) -> Self {
        Self {
            token: db.token,
            uses_allowed: db.uses_allowed,
            pending: db.pending,
            completed: db.completed,
            expiry_time: db.expires_at,
        }
    }
}

/// List all registration tokens, optionally filtered by validity
///
/// If `valid` is Some(true), only tokens that are still valid (not expired
/// and have uses remaining) are returned.
/// If `valid` is Some(false), only invalid tokens are returned.
/// If `valid` is None, all tokens are returned.
pub fn list_registration_tokens(valid: Option<bool>) -> DataResult<Vec<RegistrationTokenInfo>> {
    let mut query = user_registration_tokens::table.into_boxed();

    if let Some(is_valid) = valid {
        let now = UnixMillis::now().get() as i64;
        if is_valid {
            // Valid tokens: not expired AND (uses_allowed is null OR completed < uses_allowed)
            query = query
                .filter(
                    user_registration_tokens::expires_at
                        .is_null()
                        .or(user_registration_tokens::expires_at.gt(now)),
                )
                .filter(
                    user_registration_tokens::uses_allowed
                        .is_null()
                        .or(user_registration_tokens::completed
                            .lt(user_registration_tokens::uses_allowed.assume_not_null())),
                );
        } else {
            // Invalid tokens: expired OR (uses_allowed is not null AND completed >= uses_allowed)
            query = query.filter(
                user_registration_tokens::expires_at
                    .is_not_null()
                    .and(user_registration_tokens::expires_at.le(now))
                    .or(user_registration_tokens::uses_allowed
                        .is_not_null()
                        .and(
                            user_registration_tokens::completed
                                .ge(user_registration_tokens::uses_allowed.assume_not_null()),
                        )),
            );
        }
    }

    let tokens = query
        .order(user_registration_tokens::created_at.desc())
        .load::<DbRegistrationToken>(&mut connect()?)?;

    Ok(tokens.into_iter().map(Into::into).collect())
}

/// Get a single registration token by its token string
pub fn get_registration_token(token: &str) -> DataResult<Option<RegistrationTokenInfo>> {
    let result = user_registration_tokens::table
        .filter(user_registration_tokens::token.eq(token))
        .first::<DbRegistrationToken>(&mut connect()?)
        .optional()?;

    Ok(result.map(Into::into))
}

/// Create a new registration token
///
/// Returns true if created successfully, false if token already exists
pub fn create_registration_token(
    token: &str,
    uses_allowed: Option<i64>,
    expires_at: Option<i64>,
) -> DataResult<bool> {
    let new_token = NewDbRegistrationToken::new(token.to_owned(), uses_allowed, expires_at);

    let result = diesel::insert_into(user_registration_tokens::table)
        .values(&new_token)
        .on_conflict(user_registration_tokens::token)
        .do_nothing()
        .execute(&mut connect()?)?;

    Ok(result > 0)
}

/// Update a registration token
///
/// Returns the updated token info if found, None if token doesn't exist
pub fn update_registration_token(
    token: &str,
    uses_allowed: Option<Option<i64>>,
    expires_at: Option<Option<i64>>,
) -> DataResult<Option<RegistrationTokenInfo>> {
    // First check if the token exists
    let existing = user_registration_tokens::table
        .filter(user_registration_tokens::token.eq(token))
        .first::<DbRegistrationToken>(&mut connect()?)
        .optional()?;

    if existing.is_none() {
        return Ok(None);
    }

    // Build update based on what was provided
    let mut conn = connect()?;

    if let Some(new_uses_allowed) = uses_allowed {
        diesel::update(user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)))
            .set(user_registration_tokens::uses_allowed.eq(new_uses_allowed))
            .execute(&mut conn)?;
    }

    if let Some(new_expires_at) = expires_at {
        diesel::update(user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)))
            .set(user_registration_tokens::expires_at.eq(new_expires_at))
            .execute(&mut conn)?;
    }

    // Return updated token
    get_registration_token(token)
}

/// Delete a registration token
///
/// Returns true if deleted, false if token didn't exist
pub fn delete_registration_token(token: &str) -> DataResult<bool> {
    let result = diesel::delete(
        user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)),
    )
    .execute(&mut connect()?)?;

    Ok(result > 0)
}

/// Generate a random registration token string
///
/// Generates a cryptographically random string of the specified length
/// using only allowed characters (A-Za-z0-9._~-)
pub fn generate_token(length: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._~-";

    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Check if a token string contains only valid characters
pub fn is_valid_token_chars(token: &str) -> bool {
    token.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '~' || c == '-')
}

/// Check if a registration token is valid for use
///
/// A token is valid if:
/// - It exists
/// - It has not expired (or has no expiry)
/// - It has uses remaining (or has unlimited uses)
pub fn is_token_valid(token: &str) -> DataResult<bool> {
    let db_token = user_registration_tokens::table
        .filter(user_registration_tokens::token.eq(token))
        .first::<DbRegistrationToken>(&mut connect()?)
        .optional()?;

    let Some(db_token) = db_token else {
        return Ok(false);
    };

    let now = UnixMillis::now().get() as i64;

    // Check expiry
    if let Some(expires_at) = db_token.expires_at {
        if now >= expires_at {
            return Ok(false);
        }
    }

    // Check uses
    if let Some(uses_allowed) = db_token.uses_allowed {
        if db_token.completed >= uses_allowed {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Increment the pending count for a token (when registration starts)
pub fn increment_pending(token: &str) -> DataResult<bool> {
    let result = diesel::update(
        user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)),
    )
    .set(user_registration_tokens::pending.eq(user_registration_tokens::pending + 1))
    .execute(&mut connect()?)?;

    Ok(result > 0)
}

/// Complete a registration (decrement pending, increment completed)
pub fn complete_registration(token: &str) -> DataResult<bool> {
    let result = diesel::update(
        user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)),
    )
    .set((
        user_registration_tokens::pending.eq(user_registration_tokens::pending - 1),
        user_registration_tokens::completed.eq(user_registration_tokens::completed + 1),
    ))
    .execute(&mut connect()?)?;

    Ok(result > 0)
}

/// Cancel a pending registration (decrement pending)
pub fn cancel_registration(token: &str) -> DataResult<bool> {
    let result = diesel::update(
        user_registration_tokens::table.filter(user_registration_tokens::token.eq(token)),
    )
    .set(user_registration_tokens::pending.eq(user_registration_tokens::pending - 1))
    .execute(&mut connect()?)?;

    Ok(result > 0)
}
