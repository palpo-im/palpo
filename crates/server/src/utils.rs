use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use rand::prelude::*;

use crate::core::OwnedUserId;
use crate::core::signatures::Ed25519KeyPair;
use crate::{AppError, AppResult};

pub mod fs;
pub mod hash;
pub use hash::*;
pub mod time;
pub use time::*;
pub mod stream;
pub mod string;
pub use stream::*;
pub mod content_disposition;
mod mutex_map;
pub use mutex_map::{MutexMap, MutexMapGuard};
mod sequm_queue;
pub use sequm_queue::*;
mod defer;
pub mod sys;

#[macro_export]
macro_rules! join_path {
    ($($part:expr),+) => {
        {
            let mut p = std::path::PathBuf::new();
            $(
                p.push($part);
            )*
            path_slash::PathBufExt::to_slash_lossy(&p).to_string()
        }
    }
}

#[macro_export]
macro_rules! extract_variant {
    ($e:expr, $variant:path) => {
        match $e {
            $variant(value) => Some(value),
            _ => None,
        }
    };
}

pub fn select_config_path() -> &'static str {
    if cfg!(windows) {
        "palpo.toml"
    } else {
        const CANDIDATE_PATHS: [&str; 3] = [
            "palpo.toml",
            "/etc/palpo/palpo.toml",
            "/var/palpo/palpo.toml",
        ];
        CANDIDATE_PATHS
            .into_iter()
            .find(|path| std::fs::exists(path).unwrap_or(false))
            .unwrap_or("palpo.toml")
    }
}

pub fn shuffle<T>(vec: &mut [T]) {
    let mut rng = rand::rng();
    vec.shuffle(&mut rng);
}

pub fn increment(old: Option<&[u8]>) -> Option<Vec<u8>> {
    let number = match old.map(|bytes| bytes.try_into()) {
        Some(Ok(bytes)) => {
            let number = u64::from_be_bytes(bytes);
            number + 1
        }
        _ => 1, // Start at one. since 0 should return the first event in the db
    };

    Some(number.to_be_bytes().to_vec())
}

pub fn generate_keypair() -> Ed25519KeyPair {
    let key_content = Ed25519KeyPair::generate().unwrap();
    Ed25519KeyPair::from_der(&key_content, random_string(8))
        .unwrap_or_else(|_| panic!("{:?}", &key_content))
}

/// Parses the bytes into an u64.
pub fn u64_from_bytes(bytes: &[u8]) -> Result<u64, std::array::TryFromSliceError> {
    let array: [u8; 8] = bytes.try_into()?;
    Ok(u64::from_be_bytes(array))
}
pub fn i64_from_bytes(bytes: &[u8]) -> Result<i64, std::array::TryFromSliceError> {
    let array: [u8; 8] = bytes.try_into()?;
    Ok(i64::from_be_bytes(array))
}

/// Parses the bytes into a string.
pub fn string_from_bytes(bytes: &[u8]) -> Result<String, std::string::FromUtf8Error> {
    String::from_utf8(bytes.to_vec())
}

/// Parses a OwnedUserId from bytes.
pub fn user_id_from_bytes(bytes: &[u8]) -> AppResult<OwnedUserId> {
    OwnedUserId::try_from(
        string_from_bytes(bytes)
            .map_err(|_| AppError::public("Failed to parse string from bytes"))?,
    )
    .map_err(|_| AppError::public("Failed to parse user id from bytes"))
}

pub fn random_string(length: usize) -> String {
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub fn common_elements(
    mut iterators: impl Iterator<Item = impl Iterator<Item = Vec<u8>>>,
    check_order: impl Fn(&[u8], &[u8]) -> Ordering,
) -> Option<impl Iterator<Item = Vec<u8>>> {
    let first_iterator = iterators.next()?;
    let mut other_iterators = iterators.map(|i| i.peekable()).collect::<Vec<_>>();

    Some(first_iterator.filter(move |target| {
        other_iterators.iter_mut().all(|it| {
            while let Some(element) = it.peek() {
                match check_order(element, target) {
                    Ordering::Greater => return false, // We went too far
                    Ordering::Equal => return true,    // Element is in both iters
                    Ordering::Less => {
                        // Keep searching
                        it.next();
                    }
                }
            }
            false
        })
    }))
}

pub fn deserialize_from_str<
    'de,
    D: serde::de::Deserializer<'de>,
    T: FromStr<Err = E>,
    E: std::fmt::Display,
>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct Visitor<T: FromStr<Err = E>, E>(std::marker::PhantomData<T>);
    impl<'de, T: FromStr<Err = Err>, Err: std::fmt::Display> serde::de::Visitor<'de>
        for Visitor<T, Err>
    {
        type Value = T;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "a parsable string")
        }
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.parse().map_err(serde::de::Error::custom)
        }
    }
    deserializer.deserialize_str(Visitor(std::marker::PhantomData))
}

// Copied from librustdoc:
// https://github.com/rust-lang/rust/blob/cbaeec14f90b59a91a6b0f17fc046c66fa811892/src/librustdoc/html/escape.rs

/// Wrapper struct which will emit the HTML-escaped version of the contained
/// string when passed to a format string.
pub struct HtmlEscape<'a>(pub &'a str);

impl<'a> fmt::Display for HtmlEscape<'a> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Because the internet is always right, turns out there's not that many
        // characters to escape: http://stackoverflow.com/questions/7381974
        let HtmlEscape(s) = *self;
        let pile_o_bits = s;
        let mut last = 0;
        for (i, ch) in s.char_indices() {
            let s = match ch {
                '>' => "&gt;",
                '<' => "&lt;",
                '&' => "&amp;",
                '\'' => "&#39;",
                '"' => "&quot;",
                _ => continue,
            };
            fmt.write_str(&pile_o_bits[last..i])?;
            fmt.write_str(s)?;
            // NOTE: we only expect single byte characters here - which is fine as long as we
            // only match single byte characters
            last = i + 1;
        }

        if last < s.len() {
            fmt.write_str(&pile_o_bits[last..])?;
        }
        Ok(())
    }
}

pub fn usize_to_i64(value: usize) -> i64 {
    if value as u64 <= i64::MAX as u64 {
        value as i64
    } else {
        i64::MAX
    }
}
pub fn u64_to_i64(value: u64) -> i64 {
    if value <= i64::MAX as u64 {
        value as i64
    } else {
        i64::MAX
    }
}
