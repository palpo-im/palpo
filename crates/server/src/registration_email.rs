//! Palpo owns verification; AgentMail is only the mail transport.
use data::user::registration_email as store;
use hmac::{Hmac, KeyInit, Mac};
use salvo::prelude::*;
use serde::Deserialize;
use sha2::Sha256;

use crate::{AppError, AppResult, JsonResult, MatrixError, config, data, json_ok};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn digest(key: &str, context: &str, value: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(context.as_bytes());
    mac.update(&[0]);
    mac.update(value.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn secret_hash(secret: &str) -> AppResult<String> {
    let conf = config::get();
    let email = conf
        .registration_email
        .as_ref()
        .ok_or_else(|| MatrixError::forbidden("Email registration is unavailable.", None))?;
    Ok(digest(
        &config::RegistrationEmailConfig::read_secret(&email.otp_secret_file)?,
        "client",
        secret,
    ))
}
fn enabled() -> AppResult<config::RegistrationEmailConfig> {
    let conf = config::get();
    if (!conf.allow_registration && conf.registration_token.is_none())
        || conf.enabled_delegated_auth().is_some()
    {
        return Err(MatrixError::forbidden("Local registration is unavailable.", None).into());
    }
    conf.registration_email
        .clone()
        .ok_or_else(|| MatrixError::forbidden("Email registration is unavailable.", None).into())
}
pub fn normalize_email(value: &str) -> AppResult<String> {
    let email = value.trim().to_ascii_lowercase();
    let valid = email.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && local.len() <= 64
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains("..")
            && local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&b))
            && domain.contains('.')
            && domain.split('.').all(|s| {
                !s.is_empty()
                    && !s.starts_with('-')
                    && !s.ends_with('-')
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    });
    if !valid || email.len() > 254 {
        return Err(MatrixError::invalid_param("Enter a valid email address.").into());
    }
    Ok(email)
}
fn validate_secret(value: &str) -> AppResult<()> {
    if !(16..=255).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._=-".contains(&b))
    {
        return Err(MatrixError::invalid_param("Invalid client secret.").into());
    }
    Ok(())
}

#[handler]
pub async fn discovery() -> JsonResult<serde_json::Value> {
    let conf = config::get();
    json_ok(
        serde_json::json!({"email_otp": enabled().is_ok(), "code_length":6, "resend_after_ms":store::RESEND_MS, "registration_token":conf.registration_token.is_some()}),
    )
}

#[derive(Deserialize)]
struct SendRequest {
    email: String,
    client_secret: String,
    send_attempt: i64,
}
#[handler]
pub async fn request_token(req: &mut Request) -> JsonResult<serde_json::Value> {
    crate::hoops::check_email_send_rate(req)?;
    let conf = enabled()?;
    let body: SendRequest = req.parse_json().await?;
    validate_secret(&body.client_secret)?;
    if body.send_attempt < 1 {
        return Err(MatrixError::invalid_param("send_attempt must be positive.").into());
    }
    let email = normalize_email(&body.email)?;
    let sid = crate::utils::random_string(32);
    let code = format!("{:06}", rand::random_range(0..1_000_000u32));
    let created_at = now();
    let key = config::RegistrationEmailConfig::read_secret(&conf.otp_secret_file)?;
    let candidate = store::EmailSession {
        sid: sid.clone(),
        email: email.clone(),
        client_secret_hash: digest(&key, "client", &body.client_secret),
        code_hash: digest(&key, &sid, &code),
        send_attempt: body.send_attempt,
        created_at,
        expires_at: created_at + store::CODE_LIFETIME_MS,
        sent_at: None,
        failed_attempts: 0,
        verified_at: None,
        claimed_session: None,
        claimed_user_id: None,
        consumed_at: None,
    };
    let sid = if let Some(existing) = store::reserve(&candidate).await? {
        existing
    } else {
        send(&conf, &email, &code).await?;
        store::delivered(&sid, now()).await?;
        sid
    };
    json_ok(
        serde_json::json!({"sid":sid,"submit_url":format!("{}/_matrix/client/v3/register/email/submitToken", config::get().well_known_client().trim_end_matches('/')), "resend_after_ms":store::RESEND_MS}),
    )
}

pub async fn send(
    conf: &config::RegistrationEmailConfig,
    email: &str,
    code: &str,
) -> AppResult<()> {
    let mut url = url::Url::parse(&conf.agentmail_api_url)?;
    url.path_segments_mut()
        .map_err(|_| AppError::internal("Invalid AgentMail URL"))?
        .pop_if_empty()
        .extend(["inboxes", &conf.agentmail_inbox, "messages", "send"]);
    let key = config::RegistrationEmailConfig::read_secret(&conf.agentmail_api_key_file)?;
    let response = reqwest::Client::builder().timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none()).build()?
        .post(url).bearer_auth(key).json(&serde_json::json!({
            "to":[email], "subject":"Your Palpo verification code",
            "text":format!("Your verification code is {code}.\n\nIt expires in 10 minutes. Enter it in Rinx to continue creating your account.\nIf you did not request this code, ignore this email.\n\n您的邮箱验证码是 {code}，10 分钟内有效。请回到 Rinx 输入验证码。")
        })).send().await.map_err(|_| MatrixError::unknown("Could not send the verification email. Please wait a minute and retry."))?;
    if !response.status().is_success() {
        // Do not expose provider responses (they can contain recipient or credentials).
        tracing::warn!(status = %response.status(), "Registration email provider rejected a send");
        return Err(MatrixError::unknown(
            "Could not send the verification email. Please wait a minute and retry.",
        )
        .into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct VerifyRequest {
    sid: String,
    client_secret: String,
    token: String,
}
#[handler]
pub async fn submit_token(req: &mut Request) -> JsonResult<serde_json::Value> {
    crate::hoops::check_email_verify_rate(req)?;
    let conf = enabled()?;
    let body: VerifyRequest = req.parse_json().await?;
    validate_secret(&body.client_secret)?;
    if body.sid.len() > 128
        || body.token.len() != 6
        || !body.token.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(MatrixError::invalid_param("Enter the six-digit email code.").into());
    }
    let key = config::RegistrationEmailConfig::read_secret(&conf.otp_secret_file)?;
    if !store::verify(
        &body.sid,
        &digest(&key, "client", &body.client_secret),
        &digest(&key, &body.sid, &body.token),
        now(),
    )
    .await?
    {
        return Err(MatrixError::forbidden(
            "Code incorrect or expired. Check the latest email or request a new code.",
            None,
        )
        .into());
    }
    json_ok(serde_json::json!({"success":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_validation_and_keyed_codes() {
        assert_eq!(
            normalize_email(" Name+tag@Example.org ").unwrap(),
            "name+tag@example.org"
        );
        for email in [
            "a@b",
            "a@b@c.org",
            "a\r\nb@example.org",
            ".a@example.org",
            "a..b@example.org",
            "a@-bad.org",
        ] {
            assert!(normalize_email(email).is_err(), "{email}");
        }
        assert_ne!(
            digest("key", "session1", "123456"),
            digest("key", "session2", "123456")
        );
        assert_ne!(
            digest("key", "session1", "123456"),
            digest("other", "session1", "123456")
        );
    }
}
