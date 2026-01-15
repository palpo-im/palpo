use serde::Deserialize;

use crate::core::serde::default_true;
use crate::macros::config_example;

#[config_example(filename = "palpo-example.toml", section = "jwt")]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct JwtConfig {
    /// Enable JWT logins
    ///
    /// default: false
    #[serde(default = "default_true")]
    pub enable: bool,

    /// Validation secret key. The type of key can be configured in 'format', but defaults to the
    /// common HMAC which is a plaintext shared-secret, so you should keep this value private.
    ///
    /// display: sensitive
    /// default:
    #[serde(default)]
    pub secret: String,

    /// Format of the 'key'. Only HMAC, ECDSA, and B64HMAC are supported
    /// Binary keys cannot be pasted into this config, so B64HMAC is an
    /// alternative to HMAC for properly random secret strings.
    /// - HMAC is a plaintext shared-secret private-key.
    /// - B64HMAC is a base64-encoded version of HMAC.
    /// - ECDSA is a PEM-encoded public-key.
    ///
    /// default: "HMAC"
    #[serde(default = "default_jwt_format")]
    pub format: String,

    /// Automatically create new user from a valid claim, otherwise access is
    /// denied for an unknown even with an authentic token.
    ///
    /// default: true
    #[serde(default = "crate::core::serde::default_true")]
    pub register_user: bool,

    /// JWT algorithm
    ///
    /// default: "HS256"
    #[serde(default = "default_jwt_algorithm")]
    pub algorithm: String,

    /// Optional audience claim list. The token must claim one or more values
    /// from this list when set.
    ///
    /// default: []
    #[serde(default)]
    pub audience: Vec<String>,

    /// Optional issuer claim list. The token must claim one or more values
    /// from this list when set.
    ///
    /// default: []
    #[serde(default)]
    pub issuer: Vec<String>,

    /// Require expiration claim in the token. This defaults to false for
    /// synapse migration compatibility.
    ///
    /// default: false
    #[serde(default)]
    pub require_exp: bool,

    /// Require not-before claim in the token. This defaults to false for
    /// synapse migration compatibility.
    ///
    /// default: false
    #[serde(default)]
    pub require_nbf: bool,

    /// Validate expiration time of the token when present. Whether or not it is
    /// required depends on require_exp, but when present this ensures the token
    /// is not used after a time.
    ///
    /// default: true
    #[serde(default = "crate::core::serde::default_true")]
    pub validate_exp: bool,

    /// Validate not-before time of the token when present. Whether or not it is
    /// required depends on require_nbf, but when present this ensures the token
    /// is not used before a time.
    ///
    /// default: true
    #[serde(default = "crate::core::serde::default_true")]
    pub validate_nbf: bool,

    /// Bypass validation for diagnostic/debug use only.
    ///
    /// default: true
    #[serde(default = "crate::core::serde::default_true")]
    pub validate_signature: bool,
}

fn default_jwt_algorithm() -> String {
    "HS256".to_owned()
}
fn default_jwt_format() -> String {
    "HMAC".to_owned()
}
