//! A URI that should be a Matrix-spec compliant [MXC URI].
//!
//! [MXC URI]: https://spec.matrix.org/latest/client-server-api/#matrix-content-mxc-uris

use std::fmt;
use std::num::NonZeroU8;

use palpo_identifiers_validation::error::MxcUriError;
use palpo_identifiers_validation::mxc_uri::validate;
use serde::{Serialize, Serializer};

use super::ServerName;
use crate::macros::IdDst;

type Result<T, E = MxcUriError> = std::result::Result<T, E>;

/// A URI that should be a Matrix-spec compliant [MXC URI].
///
/// [MXC URI]: https://spec.matrix.org/latest/client-server-api/#matrix-content-mxc-uris
#[repr(transparent)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, IdDst)]
pub struct MxcUri(str);

impl MxcUri {
    /// If this is a valid MXC URI, returns the media ID.
    pub fn media_id(&self) -> Result<&str> {
        self.parts().map(|mxc| mxc.media_id)
    }

    /// If this is a valid MXC URI, returns the server name.
    pub fn server_name(&self) -> Result<&ServerName> {
        self.parts().map(|mxc| mxc.server_name)
    }

    /// If this is a valid MXC URI, returns a `(server_name, media_id)` tuple,
    /// else it returns the error.
    pub fn parts(&self) -> Result<Mxc<'_>> {
        self.extract_slash_idx().map(|idx| Mxc::<'_> {
            server_name: ServerName::from_borrowed(&self.as_str()[6..idx.get() as usize]),
            media_id: &self.as_str()[idx.get() as usize + 1..],
        })
    }

    /// Validates the URI and returns an error if it failed.
    pub fn validate(&self) -> Result<()> {
        self.extract_slash_idx().map(|_| ())
    }

    /// Convenience method for `.validate().is_ok()`.
    #[inline(always)]
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    // convenience method for calling validate(self)
    #[inline(always)]
    fn extract_slash_idx(&self) -> Result<NonZeroU8> {
        validate(self.as_str())
    }
}

/// Structured MXC URI which may reference strings from separate sources without
/// serialization
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(clippy::exhaustive_structs)]
pub struct Mxc<'a> {
    /// ServerName part of the MXC URI
    pub server_name: &'a ServerName,

    /// MediaId part of the MXC URI
    pub media_id: &'a str,
}
impl fmt::Display for Mxc<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mxc://{}/{}", self.server_name, self.media_id)
    }
}

impl<'a> TryFrom<&'a MxcUri> for Mxc<'a> {
    type Error = MxcUriError;

    fn try_from(s: &'a MxcUri) -> Result<Self, Self::Error> {
        s.parts()
    }
}

impl<'a> TryFrom<&'a str> for Mxc<'a> {
    type Error = MxcUriError;

    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        let s: &MxcUri = s.into();
        s.try_into()
    }
}

impl<'a> TryFrom<&'a OwnedMxcUri> for Mxc<'a> {
    type Error = MxcUriError;

    fn try_from(s: &'a OwnedMxcUri) -> Result<Self, Self::Error> {
        let s: &MxcUri = s.as_ref();
        s.try_into()
    }
}

impl Serialize for Mxc<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.to_string().as_str())
    }
}

#[cfg(test)]
mod tests {
    use palpo_identifiers_validation::error::MxcUriError;

    use super::MxcUri;

    // #[test]
    // fn parse_mxc_uri() {
    //     let mxc = Box::<MxcUri>::from("mxc://127.0.0.1/asd32asdfasdsd");

    //     assert!(mxc.is_valid());
    //     assert_eq!(
    //         mxc.parts(),
    //         Ok((
    //             "127.0.0.1".try_into().expect("Failed to create ServerName"),
    //             "asd32asdfasdsd"
    //         ))
    //     );
    // }

    #[test]
    fn parse_mxc_uri_without_media_id() {
        let mxc = Box::<MxcUri>::from("mxc://127.0.0.1");

        assert!(!mxc.is_valid());
        assert_eq!(mxc.parts(), Err(MxcUriError::MissingSlash));
    }

    #[test]
    fn parse_mxc_uri_without_protocol() {
        assert!(!Box::<MxcUri>::from("127.0.0.1/asd32asdfasdsd").is_valid());
    }

    #[test]
    fn serialize_mxc_uri() {
        assert_eq!(
            serde_json::to_string(&Box::<MxcUri>::from("mxc://server/1234id"))
                .expect("Failed to convert MxcUri to JSON."),
            r#""mxc://server/1234id""#
        );
    }

    // #[test]
    // fn deserialize_mxc_uri() {
    //     let mxc =
    //         serde_json::from_str::<OwnedMxcUri>(r#""mxc://server/1234id""#).expect("Failed to
    // convert JSON to MxcUri");

    //     assert_eq!(mxc.as_str(), "mxc://server/1234id");
    //     assert!(mxc.is_valid());
    //     assert_eq!(
    //         mxc.parts(),
    //         Ok(("server".try_into().expect("Failed to create ServerName"), "1234id"))
    //     );
    // }
}
