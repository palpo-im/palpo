use serde::{Deserialize, Serialize};

use crate::Error;

// These values cross JSON into the OctoScript host. Larger u64 values must not
// silently round in a consumer that represents JSON numbers as doubles.
pub const MAX_EXACT_JSON_INTEGER: u64 = (1 << 53) - 1;

/// SHA-256 of a canonical, immutable operation definition. The owning adapter
/// computes it; accepting this value is not verification of a signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DefinitionDigest(String);

impl TryFrom<String> for DefinitionDigest {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::InvalidIdentifier);
        }
        Ok(Self(value))
    }
}

impl From<DefinitionDigest> for String {
    fn from(value: DefinitionDigest) -> Self {
        value.0
    }
}

macro_rules! opaque_id {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl TryFrom<String> for $name {
            type Error = Error;
            fn try_from(value: String) -> Result<Self, Error> {
                if value.is_empty() || value.len() > 128
                    || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(Error::InvalidIdentifier);
                }
                Ok(Self(value))
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self { value.0 }
        }
        impl $name {
            pub fn as_str(&self) -> &str { &self.0 }
        }
    )+};
}

// Deliberately distinct types: legacy Hagency Engagement IDs denote individual
// agent allocations. A server hostname cannot identify a server engagement.
opaque_id!(
    ServerEngagementId,
    ProjectId,
    ResourceAllocationId,
    AgentAllocationId,
    RequestId,
    CommandId
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServerName(String);

impl TryFrom<String> for ServerName {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        if value.len() > 255 {
            return Err(Error::InvalidIdentifier);
        }
        palpo_identifiers_validation::server_name::validate(&value)
            .map_err(|_| Error::InvalidIdentifier)?;
        Ok(Self(value))
    }
}

impl From<ServerName> for String {
    fn from(value: ServerName) -> Self {
        value.0
    }
}

impl ServerName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MatrixUserId(String);

impl TryFrom<String> for MatrixUserId {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        // The existing compatibility validator accepts empty/whitespace
        // localparts. Business-role bindings require a real nonempty ID.
        if value.len() > 255
            || !value.starts_with('@')
            || value.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(Error::InvalidIdentifier);
        }
        let (local, _) = value.split_once(':').ok_or(Error::InvalidIdentifier)?;
        if local.len() <= 1 {
            return Err(Error::InvalidIdentifier);
        }
        palpo_identifiers_validation::user_id::validate(&value)
            .map_err(|_| Error::InvalidIdentifier)?;
        Ok(Self(value))
    }
}

impl From<MatrixUserId> for String {
    fn from(value: MatrixUserId) -> Self {
        value.0
    }
}

impl MatrixUserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn belongs_to(&self, server: &ServerName) -> bool {
        self.0
            .split_once(':')
            .is_some_and(|(_, name)| name == server.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Revision(u64);

impl TryFrom<u64> for Revision {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self, Error> {
        if value == 0 || value > MAX_EXACT_JSON_INTEGER {
            return Err(Error::InvalidNumber);
        }
        Ok(Self(value))
    }
}

impl From<Revision> for u64 {
    fn from(value: Revision) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct Tokens(u64);

impl TryFrom<u64> for Tokens {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self, Error> {
        if value > MAX_EXACT_JSON_INTEGER {
            return Err(Error::InvalidNumber);
        }
        Ok(Self(value))
    }
}

impl From<Tokens> for u64 {
    fn from(value: Tokens) -> Self {
        value.0
    }
}
