#![doc(html_favicon_url = "https://palpo.io/favicon.ico")]
#![doc(html_logo_url = "https://palpo.io/images/logo.png")]

pub mod client_secret;
pub mod device_key_id;
pub mod error;
pub mod event_id;
pub mod key_id;
pub mod mxc_uri;
pub mod room_alias_id;
pub mod room_id;
pub mod room_id_or_alias_id;
pub mod room_version_id;
pub mod server_name;
pub mod server_signing_key_version;
pub mod user_id;
pub mod voip_version_id;

pub use error::Error;

/// All identifiers must be 255 bytes or less.
pub const MAX_BYTES: usize = 255;

/// Checks if an identifier is valid.
fn validate_id(id: &str, first_byte: u8) -> Result<(), Error> {
    #[cfg(not(feature = "compat-arbitrary-length-ids"))]
    if id.len() > MAX_BYTES {
        return Err(Error::MaximumLengthExceeded);
    }

    if id.as_bytes().first() != Some(&first_byte) {
        return Err(Error::MissingLeadingSigil);
    }

    Ok(())
}

/// Checks an identifier that contains a localpart and hostname for validity.
fn parse_id(id: &str, first_byte: u8) -> Result<usize, Error> {
    validate_id(id, first_byte)?;
    let colon_idx = id.find(':').ok_or(Error::MissingColon)?;
    server_name::validate(&id[colon_idx + 1..])?;
    Ok(colon_idx)
}

/// Checks an identifier that contains a localpart and hostname for validity.
fn validate_delimited_id(id: &str, first_byte: u8) -> Result<(), Error> {
    parse_id(id, first_byte)?;
    Ok(())
}

/// Helper trait to validate the name of a key.
pub trait KeyName: AsRef<str> {
    /// Validate the given string for this name.
    fn validate(s: &str) -> Result<(), Error>;
}

/// Check whether the Matrix identifier localpart is [allowed over federation].
///
/// According to the spec, localparts can consist of any legal non-surrogate Unicode code points
/// except for `:` and `NUL` (`U+0000`).
///
/// [allowed over federation]: https://spec.matrix.org/latest/appendices/#historical-user-ids
pub fn localpart_is_backwards_compatible(localpart: &str) -> Result<(), Error> {
    let is_invalid = localpart.contains([':', '\0']);
    if is_invalid {
        Err(Error::InvalidCharacters)
    } else {
        Ok(())
    }
}
