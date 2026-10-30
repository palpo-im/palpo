use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead;
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};

use super::ThreadCursor;
use crate::core::client::room::IncludeThreads;
use crate::core::{RoomId, UnixMillis, UserId};
use crate::{AppError, AppResult, MatrixError};

const NONCE_LENGTH: usize = 12;
const TOKEN_TTL_MS: i64 = 60 * 60 * 1000;
const CLOCK_SKEW_MS: i64 = 30_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedCursor {
    cursor: ThreadCursor,
    issued_at: i64,
    expires_at: i64,
}

pub(super) fn invalid() -> AppError {
    MatrixError::invalid_param("Invalid thread pagination token.").into()
}

fn aad(room: &RoomId, user: &UserId, include: &IncludeThreads) -> AppResult<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "palpo/threads",
        "t2/aes256gcm/v1",
        room.as_str(),
        user.as_str(),
        include.to_string(),
        "last_sn.desc,event_id.desc",
    ))?)
}

pub(super) fn encode(
    cursor: ThreadCursor,
    room: &RoomId,
    user: &UserId,
    include: &IncludeThreads,
) -> AppResult<String> {
    let key = crate::config::keypair().derive_secret(b"palpo/thread-pagination/t2/aes256gcm/v1");
    seal(
        &key,
        cursor,
        &aad(room, user, include)?,
        UnixMillis::now().get() as i64,
    )
}

pub(super) fn decode(
    token: &str,
    room: &RoomId,
    user: &UserId,
    include: &IncludeThreads,
) -> AppResult<ThreadCursor> {
    let key = crate::config::keypair().derive_secret(b"palpo/thread-pagination/t2/aes256gcm/v1");
    open(
        &key,
        token,
        &aad(room, user, include)?,
        room,
        UnixMillis::now().get() as i64,
    )
}

fn seal(key: &[u8; 32], cursor: ThreadCursor, aad: &[u8], now: i64) -> AppResult<String> {
    let mut nonce = [0; NONCE_LENGTH];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| AppError::internal("unable to generate thread pagination nonce"))?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key)
            .map_err(|_| AppError::internal("invalid thread pagination encryption key"))?,
    );
    let mut payload = serde_json::to_vec(&SealedCursor {
        cursor,
        issued_at: now,
        expires_at: now.checked_add(TOKEN_TTL_MS).ok_or_else(invalid)?,
    })?;
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(aad),
        &mut payload,
    )
    .map_err(|_| AppError::internal("unable to encrypt thread pagination token"))?;
    let bytes = [nonce.as_slice(), payload.as_slice()].concat();
    Ok(format!("t2.{}", URL_SAFE_NO_PAD.encode(bytes)))
}

fn open(
    key: &[u8; 32],
    token: &str,
    aad: &[u8],
    room: &RoomId,
    now: i64,
) -> AppResult<ThreadCursor> {
    // Bound allocation before decoding or invoking the AEAD implementation.
    if token.len() > 4096 {
        return Err(invalid());
    }
    let mut bytes = URL_SAFE_NO_PAD
        .decode(token.strip_prefix("t2.").ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if bytes.len() < NONCE_LENGTH + aead::AES_256_GCM.tag_len() {
        return Err(invalid());
    }
    let nonce: [u8; NONCE_LENGTH] = bytes[..NONCE_LENGTH].try_into().map_err(|_| invalid())?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key).map_err(|_| invalid())?,
    );
    let plaintext = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(aad),
            &mut bytes[NONCE_LENGTH..],
        )
        .map_err(|_| invalid())?;
    // Authenticate the complete ciphertext before parsing any attacker input.
    let payload: SealedCursor = serde_json::from_slice(plaintext).map_err(|_| invalid())?;
    if payload.cursor.room_id != room
        || payload.cursor.last_sn < 0
        || payload.issued_at < 0
        || payload.issued_at > now.saturating_add(CLOCK_SKEW_MS)
        || payload.expires_at <= now
        || payload.issued_at.checked_add(TOKEN_TTL_MS) != Some(payload.expires_at)
    {
        return Err(invalid());
    }
    Ok(payload.cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor() -> ThreadCursor {
        ThreadCursor {
            room_id: crate::core::room_id!("!thread:example.test").to_owned(),
            event_id: crate::core::event_id!("$hidden-thread-root").to_owned(),
            last_sn: 123,
        }
    }

    #[test]
    fn encrypted_thread_cursors_bind_context_and_hide_internal_positions() {
        let room = crate::core::room_id!("!thread:example.test");
        let user = crate::core::user_id!("@reader:example.test");
        let context = aad(room, user, &IncludeThreads::All).unwrap();
        let first = seal(&[7; 32], cursor(), &context, 100_000).unwrap();
        let second = seal(&[7; 32], cursor(), &context, 100_000).unwrap();
        assert_ne!(first, second, "each token needs a fresh random nonce");
        let wire = URL_SAFE_NO_PAD
            .decode(first.strip_prefix("t2.").unwrap())
            .unwrap();
        assert!(
            !wire
                .windows(b"$hidden-thread-root".len())
                .any(|w| w == b"$hidden-thread-root")
        );
        assert!(serde_json::from_slice::<serde_json::Value>(&wire).is_err());
        let decoded = open(&[7; 32], &first, &context, room, 100_001).unwrap();
        assert_eq!(decoded.last_sn, 123);
        assert_eq!(decoded.event_id, cursor().event_id);
        assert!(open(&[8; 32], &first, &context, room, 100_001).is_err());
        for wrong_context in [
            aad(
                room,
                crate::core::user_id!("@other:example.test"),
                &IncludeThreads::All,
            )
            .unwrap(),
            aad(
                crate::core::room_id!("!other:example.test"),
                user,
                &IncludeThreads::All,
            )
            .unwrap(),
            b"different-version-or-filter".to_vec(),
        ] {
            assert!(open(&[7; 32], &first, &wrong_context, room, 100_001).is_err());
        }
        let mut corrupted = wire;
        *corrupted.last_mut().unwrap() ^= 1;
        let corrupted = format!("t2.{}", URL_SAFE_NO_PAD.encode(corrupted));
        assert!(open(&[7; 32], &corrupted, &context, room, 100_001).is_err());
        assert!(open(&[7; 32], &first, &context, room, 100_000 + TOKEN_TTL_MS).is_err());
        assert!(open(&[7; 32], &first, &context, room, 0).is_err());
        for malformed in [
            "t2.invalid".to_owned(),
            "t2.".to_owned(),
            "t2.".to_owned() + &"A".repeat(4096),
        ] {
            assert!(open(&[7; 32], &malformed, &context, room, 100_001).is_err());
        }
        let mut negative = cursor();
        negative.last_sn = -1;
        let negative = seal(&[7; 32], negative, &context, 100_000).unwrap();
        assert!(open(&[7; 32], &negative, &context, room, 100_001).is_err());
    }
}
