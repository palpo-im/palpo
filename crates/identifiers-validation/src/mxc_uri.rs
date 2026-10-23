use std::num::NonZeroU16;

use crate::error::MxcUriError;
use crate::server_name;

const PROTOCOL: &str = "mxc://";

pub fn validate(uri: &str) -> Result<NonZeroU16, MxcUriError> {
    let uri = match uri.strip_prefix(PROTOCOL) {
        Some(uri) => uri,
        None => return Err(MxcUriError::WrongSchema),
    };

    let index = match uri.find('/') {
        Some(index) => index,
        None => return Err(MxcUriError::MissingSlash),
    };

    let server_name = &uri[..index];
    let media_id = &uri[index + 1..];
    // See: https://spec.matrix.org/v1.17/client-server-api/#security-considerations-5
    let media_id_is_valid = media_id
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' ));

    if !media_id_is_valid {
        Err(MxcUriError::MediaIdMalformed)
    } else if server_name::validate(server_name).is_err() {
        Err(MxcUriError::ServerNameMalformed)
    } else {
        // Server-name validation bounds the name to 261 bytes. Including the scheme,
        // the separator index is therefore nonzero and fits in a u16.
        Ok(NonZeroU16::new((index + PROTOCOL.len()) as u16).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::error::MxcUriError;

    #[test]
    fn separator_index_does_not_wrap_at_u8_boundary() {
        for len in [249, 250, 251, 255] {
            let uri = format!("mxc://{}/media", "a".repeat(len));
            let index = usize::from(validate(&uri).unwrap().get());
            assert_eq!(index, len + 6);
            assert_eq!(&uri[index..], "/media");
        }
    }

    #[test]
    fn validates_maximum_server_name_with_port() {
        let name = format!("{}:65535", "a".repeat(255));
        assert_eq!(
            usize::from(validate(&format!("mxc://{name}/media")).unwrap().get()),
            267,
        );
    }

    #[test]
    fn rejects_oversized_server_names_without_panicking() {
        for len in [262, 506, 65_536] {
            assert_eq!(
                validate(&format!("mxc://{}/media", "a".repeat(len))),
                Err(MxcUriError::ServerNameMalformed),
            );
        }
    }
}
