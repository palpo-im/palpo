//! Durable, single-use email proofs. Every state transition locks its row.
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{DbUser, NewDbPassword, NewDbProfile, NewDbUser, NewDbUserThreepid};
use crate::core::{MatrixError, UnixMillis, UserId};
use crate::schema::*;
use crate::{DataError, DataResult, connect};

pub const CODE_LIFETIME_MS: i64 = 600_000;
pub const PROOF_LIFETIME_MS: i64 = 1_800_000;
pub const RESEND_MS: i64 = 60_000;
pub const MAX_FAILURES: i32 = 5;

#[derive(QueryableByName)]
pub struct EmailSession {
    #[diesel(sql_type = Text)]
    pub sid: String,
    #[diesel(sql_type = Text)]
    pub email: String,
    #[diesel(sql_type = Text)]
    pub client_secret_hash: String,
    #[diesel(sql_type = Text)]
    pub code_hash: String,
    #[diesel(sql_type = BigInt)]
    pub send_attempt: i64,
    #[diesel(sql_type = BigInt)]
    pub created_at: i64,
    #[diesel(sql_type = BigInt)]
    pub expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub sent_at: Option<i64>,
    #[diesel(sql_type = Integer)]
    pub failed_attempts: i32,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub verified_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    pub claimed_session: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub claimed_user_id: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub consumed_at: Option<i64>,
}

fn invalid() -> DataError {
    MatrixError::forbidden(
        "Email verification expired or invalid. Request a new code.",
        None,
    )
    .into()
}
async fn lock_email(conn: &mut AsyncPgConnection, email: &str) -> DataResult<()> {
    diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 174209))")
        .bind::<Text, _>(email)
        .execute(conn)
        .await?;
    Ok(())
}

/// Returns an already delivered session for a protocol retry, otherwise reserves
/// a new send. Reservations also count failed deliveries to bound provider cost.
pub async fn reserve(candidate: &EmailSession) -> DataResult<Option<String>> {
    // Cleanup before acquiring any recipient/row locks to preserve lock order.
    diesel::sql_query("DELETE FROM registration_email_sessions WHERE created_at < $1")
        .bind::<BigInt, _>(candidate.created_at - 86_400_000)
        .execute(&mut connect().await?)
        .await?;
    connect().await?.transaction::<_,DataError,_>(async |conn| {
        lock_email(conn, &candidate.email).await?;
        let recent = diesel::sql_query("SELECT * FROM registration_email_sessions WHERE email = $1 AND created_at > $2 ORDER BY created_at DESC FOR UPDATE")
            .bind::<Text,_>(&candidate.email).bind::<BigInt,_>(candidate.created_at - 3_600_000)
            .load::<EmailSession>(conn).await?;
        if let Some(same) = recent.iter().find(|s| s.client_secret_hash == candidate.client_secret_hash && s.send_attempt == candidate.send_attempt) {
            if same.sent_at.is_some() && same.consumed_at.is_none() && same.expires_at > candidate.created_at {
                return Ok(Some(same.sid.clone()));
            }
            return Err(invalid());
        }
        if recent.len() >= 6 || recent.as_slice().first().is_some_and(|s| candidate.created_at - s.created_at < RESEND_MS) {
            return Err(MatrixError::limit_exceeded("Please wait before requesting another email code.", None).into());
        }
        if recent.iter().any(|s| s.client_secret_hash == candidate.client_secret_hash && s.send_attempt >= candidate.send_attempt) {
            return Err(invalid());
        }
        // A resend invalidates earlier codes for this client, including verified proofs.
        diesel::sql_query("UPDATE registration_email_sessions SET consumed_at = $1 WHERE email = $2 AND client_secret_hash = $3 AND consumed_at IS NULL")
            .bind::<BigInt,_>(candidate.created_at).bind::<Text,_>(&candidate.email)
            .bind::<Text,_>(&candidate.client_secret_hash).execute(conn).await?;
        diesel::sql_query("INSERT INTO registration_email_sessions (sid,email,client_secret_hash,code_hash,send_attempt,created_at,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind::<Text,_>(&candidate.sid).bind::<Text,_>(&candidate.email)
            .bind::<Text,_>(&candidate.client_secret_hash).bind::<Text,_>(&candidate.code_hash)
            .bind::<BigInt,_>(candidate.send_attempt).bind::<BigInt,_>(candidate.created_at)
            .bind::<BigInt,_>(candidate.expires_at).execute(conn).await?;
        Ok(None)
    }).await
}

pub async fn delivered(sid: &str, now: i64) -> DataResult<()> {
    diesel::sql_query("UPDATE registration_email_sessions SET sent_at = $1 WHERE sid = $2 AND consumed_at IS NULL")
        .bind::<BigInt,_>(now).bind::<Text,_>(sid).execute(&mut connect().await?).await?;
    Ok(())
}

pub async fn verify(sid: &str, secret_hash: &str, code_hash: &str, now: i64) -> DataResult<bool> {
    connect().await?.transaction::<_,DataError,_>(async |conn| {
        let row = diesel::sql_query("SELECT * FROM registration_email_sessions WHERE sid = $1 FOR UPDATE")
            .bind::<Text,_>(sid).get_result::<EmailSession>(conn).await.optional()?;
        let Some(row) = row else { return Ok(false) };
        if row.client_secret_hash != secret_hash || row.sent_at.is_none() || row.consumed_at.is_some()
            || now >= row.expires_at || row.failed_attempts >= MAX_FAILURES { return Ok(false) }
        if row.code_hash != code_hash {
            // Return Ok(false) so this attempt is committed, never rolled back.
            diesel::sql_query("UPDATE registration_email_sessions SET failed_attempts = failed_attempts + 1 WHERE sid = $1")
                .bind::<Text,_>(sid).execute(conn).await?;
            return Ok(false);
        }
        diesel::sql_query("UPDATE registration_email_sessions SET verified_at = COALESCE(verified_at, $1) WHERE sid = $2")
            .bind::<BigInt,_>(now).bind::<Text,_>(sid).execute(conn).await?;
        Ok(true)
    }).await
}

pub async fn claim(
    sid: &str,
    secret_hash: &str,
    session: &str,
    user_id: &UserId,
    now: i64,
) -> DataResult<bool> {
    let updated = diesel::sql_query("UPDATE registration_email_sessions SET claimed_session = $1, claimed_user_id = $2 WHERE sid = $3 AND client_secret_hash = $4 AND consumed_at IS NULL AND verified_at > $5 AND (claimed_session IS NULL OR (claimed_session = $1 AND claimed_user_id = $2))")
        .bind::<Text,_>(session).bind::<Text,_>(user_id.as_str()).bind::<Text,_>(sid)
        .bind::<Text,_>(secret_hash).bind::<BigInt,_>(now-PROOF_LIFETIME_MS)
        .execute(&mut connect().await?).await?;
    Ok(updated == 1)
}

pub async fn is_claimed(sid: &str, secret_hash: &str, session: &str, now: i64) -> DataResult<bool> {
    Ok(diesel::sql_query("SELECT * FROM registration_email_sessions WHERE sid = $1 AND client_secret_hash = $2 AND claimed_session = $3 AND verified_at > $4 AND consumed_at IS NULL")
        .bind::<Text,_>(sid).bind::<Text,_>(secret_hash).bind::<Text,_>(session)
        .bind::<BigInt,_>(now-PROOF_LIFETIME_MS).get_result::<EmailSession>(&mut connect().await?).await.optional()?.is_some())
}

/// Insert, never upsert: a competing registration cannot replace a password.
/// Account, password, profile, verified address and proof consumption commit together.
pub async fn create_registered_user(
    new_user: &NewDbUser,
    password_hash: Option<&str>,
    email_session: Option<&str>,
    now: i64,
) -> DataResult<DbUser> {
    connect().await?.transaction::<_,DataError,_>(async |conn| {
        let proof = if let Some(session) = email_session {
            let before = diesel::sql_query("SELECT * FROM registration_email_sessions WHERE claimed_session = $1")
                .bind::<Text,_>(session).get_result::<EmailSession>(conn).await.optional()?.ok_or_else(invalid)?;
            // Same lock order as resend: recipient first, then session row.
            lock_email(conn, &before.email).await?;
            let row = diesel::sql_query("SELECT * FROM registration_email_sessions WHERE claimed_session = $1 FOR UPDATE")
                .bind::<Text,_>(session).get_result::<EmailSession>(conn).await.optional()?.ok_or_else(invalid)?;
            if row.claimed_user_id.as_deref() != Some(new_user.id.as_str()) || row.consumed_at.is_some()
                || !row.verified_at.is_some_and(|v| now-v < PROOF_LIFETIME_MS) { return Err(invalid()) }
            if user_threepids::table.filter(user_threepids::medium.eq("email")).filter(user_threepids::address.eq(&row.email))
                .count().get_result::<i64>(conn).await? > 0 { return Err(MatrixError::forbidden("This email is already associated with an account. Sign in instead.",None).into()) }
            Some(row)
        } else { None };
        let user = diesel::insert_into(users::table).values(new_user).get_result::<DbUser>(conn).await
            .map_err(|e| match e { diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation,_) => MatrixError::user_in_use("Desired user ID is already taken.").into(), e => DataError::from(e) })?;
        diesel::insert_into(user_profiles::table).values(NewDbProfile {
            user_id: new_user.id.clone(), room_id: None, display_name: Some(new_user.localpart.clone()), avatar_url: None, blurhash: None,
        }).execute(conn).await?;
        if let Some(hash) = password_hash {
            diesel::insert_into(user_passwords::table).values(NewDbPassword { user_id: new_user.id.clone(), hash: hash.to_owned(), created_at: UnixMillis::now() }).execute(conn).await?;
        }
        if let Some(proof) = proof {
            diesel::insert_into(user_threepids::table).values(NewDbUserThreepid {
                user_id: new_user.id.clone(), medium: "email".into(), address: proof.email,
                validated_at: UnixMillis::now(), added_at: UnixMillis::now(),
            }).execute(conn).await?;
            diesel::sql_query("UPDATE registration_email_sessions SET consumed_at = $1 WHERE sid = $2")
                .bind::<BigInt,_>(now).bind::<Text,_>(proof.sid).execute(conn).await?;
        }
        Ok(user)
    }).await
}
