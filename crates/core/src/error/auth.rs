//! Errors that can be sent from the homeserver.
use std::collections::BTreeMap;

use crate::PrivOwnedStr;

/// Errors in the `WWW-Authenticate` header.
///
/// Use `::from_str()` to parse a challenge and `TryInto<http::HeaderValue>`
/// to serialize it.

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthenticateError {
    /// The bearer token is expired, revoked, malformed, or otherwise invalid.
    InvalidToken,

    /// insufficient_scope
    ///
    /// Encountered when authentication is handled by OpenID Connect and the
    /// current access token isn't authorized for the proper scope for this
    /// request. It should be paired with a `401` status code and a
    /// `M_FORBIDDEN` error.
    InsufficientScope {
        /// The new scope to request an authorization for.
        scope: String,
    },

    #[doc(hidden)]
    _Custom {
        errcode: PrivOwnedStr,
        attributes: AuthenticateAttrs,
    },
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticateAttrs(BTreeMap<String, String>);

impl AuthenticateError {
    /// Construct an `AuthenticateError` from a string.
    ///
    /// Returns `None` if the string doesn't contain an error.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        if let Some(val) = s.strip_prefix("Bearer").map(str::trim) {
            let mut errcode = None;
            let mut attrs = BTreeMap::new();

            // Split the attributes separated by commas and optionally spaces, then split
            // the keys and the values, with the values optionally surrounded by
            // double quotes.
            for (key, value) in val
                .split(',')
                .filter_map(|attr| attr.trim().split_once('='))
                .map(|(key, value)| (key, value.trim_matches('"')))
            {
                if key == "error" {
                    errcode = Some(value);
                } else {
                    attrs.insert(key.to_owned(), value.to_owned());
                }
            }

            if let Some(errcode) = errcode {
                let error = if let Some(scope) = attrs
                    .get("scope")
                    .filter(|_| errcode == "insufficient_scope")
                {
                    AuthenticateError::InsufficientScope {
                        scope: scope.to_owned(),
                    }
                } else if errcode == "invalid_token" && attrs.is_empty() {
                    AuthenticateError::InvalidToken
                } else {
                    AuthenticateError::_Custom {
                        errcode: PrivOwnedStr(errcode.into()),
                        attributes: AuthenticateAttrs(attrs),
                    }
                };

                return Some(error);
            }
        }

        None
    }
}

impl TryFrom<&AuthenticateError> for http::HeaderValue {
    type Error = http::header::InvalidHeaderValue;

    fn try_from(error: &AuthenticateError) -> Result<Self, Self::Error> {
        let s = match error {
            AuthenticateError::InvalidToken => {
                return Ok(http::HeaderValue::from_static(
                    "Bearer error=\"invalid_token\"",
                ));
            }
            AuthenticateError::InsufficientScope { scope } => {
                format!("Bearer error=\"insufficient_scope\", scope=\"{scope}\"")
            }
            AuthenticateError::_Custom {
                errcode,
                attributes,
            } => {
                let mut s = format!("Bearer error=\"{}\"", errcode.0);

                for (key, value) in attributes.0.iter() {
                    s.push_str(&format!(", {key}=\"{value}\""));
                }

                s
            }
        };

        s.try_into()
    }
}
