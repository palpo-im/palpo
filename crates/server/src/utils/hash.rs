use std::fmt::Write;
use std::path::Path;

use chksum::{SHA2_256, sha2_256};
use fast32::base32::CROCKFORD;

#[derive(Debug)]
pub struct Checksum(Vec<u8>);
impl Checksum {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_ref()
    }
    pub fn to_base32_crockford(&self) -> String {
        CROCKFORD.encode(self.as_bytes())
    }
    pub fn to_hex_uppercase(&self) -> String {
        let mut result = String::with_capacity(self.0.len() * 2);
        for b in &self.0 {
            write!(result, "{b:02X}").unwrap();
        }
        result
    }
}
pub fn base32_crockford(data: &[u8]) -> String {
    CROCKFORD.encode(data)
}

pub fn hash_file_sha2_256(file: impl AsRef<Path>) -> Result<Checksum, std::io::Error> {
    let digest = sha2_256::chksum(file.as_ref()).unwrap();
    Ok(Checksum(digest.as_bytes().to_vec()))
}
pub fn hash_data_sha2_256(data: &[u8]) -> Result<Checksum, std::io::Error> {
    let digest = sha2_256::chksum(data).unwrap();
    Ok(Checksum(digest.as_bytes().to_vec()))
}

// https://docs.rs/crate/checksums/0.6.0/source/src/hashing/mod.rs
pub fn hash_string(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(result, "{b:02X}").unwrap();
    }
    result
}

/// Calculate a new has h for the given password
pub fn hash_password(password: &str) -> Result<String, argon2::Error> {
    let hashing_conf = argon2::Config {
        variant: argon2::Variant::Argon2id,
        ..Default::default()
    };

    let salt = super::random_string(32);
    argon2::hash_encoded(password.as_bytes(), salt.as_bytes(), &hashing_conf)
}

#[tracing::instrument(skip(keys))]
pub fn hash_keys<'a, T, I>(keys: I) -> Vec<u8>
where
    I: Iterator<Item = T> + 'a,
    T: AsRef<[u8]> + 'a,
{
    let mut hash = SHA2_256::new();
    for key in keys {
        hash.update(key.as_ref());
    }
    hash.digest().as_bytes().to_vec()
}
