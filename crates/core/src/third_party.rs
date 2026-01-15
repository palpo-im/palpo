//! Common types for the [third party networks module][thirdparty].
//!
//! [thirdparty]: https://spec.matrix.org/latest/client-server-api/#third-party-networks

use std::collections::BTreeMap;

use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::serde::StringEnum;
use crate::{OwnedRoomAliasId, OwnedUserId, PrivOwnedStr, UnixMillis};

/// Metadata about a third party protocol.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Default, Debug)]
pub struct Protocol {
    /// Fields which may be used to identify a third party user.
    pub user_fields: Vec<String>,

    /// Fields which may be used to identify a third party location.
    pub location_fields: Vec<String>,

    /// A content URI representing an icon for the third party protocol.
    #[serde(default)]
    pub icon: String,

    /// The type definitions for the fields defined in `user_fields` and
    /// `location_fields`.
    pub field_types: BTreeMap<String, FieldType>,

    /// A list of objects representing independent instances of configuration.
    pub instances: Vec<ProtocolInstance>,
}

/// Metadata about an instance of a third party protocol.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct ProtocolInstance {
    /// A human-readable description for the protocol, such as the name.
    pub desc: String,

    /// An optional content URI representing the protocol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Preset values for `fields` the client may use to search by.
    pub fields: BTreeMap<String, String>,

    /// A unique identifier across all instances.
    pub network_id: String,

    /// A unique identifier across all instances.
    ///
    /// See [matrix-spec#833](https://github.com/matrix-org/matrix-spec/issues/833).
    #[cfg(feature = "unstable-unspecified")]
    pub instance_id: String,
}

/// A type definition for a field used to identify third party users or
/// locations.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct FieldType {
    /// A regular expression for validation of a field's value.
    pub regexp: String,

    /// A placeholder serving as a valid example of the field value.
    pub placeholder: String,
}

/// A third party network location.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct Location {
    /// An alias for a matrix room.
    pub alias: OwnedRoomAliasId,

    /// The protocol ID that the third party location is a part of.
    pub protocol: String,

    /// Information used to identify this third party location.
    pub fields: BTreeMap<String, String>,
}

impl Location {
    /// Creates a new `Location` with the given alias, protocol and fields.
    pub fn new(
        alias: OwnedRoomAliasId,
        protocol: String,
        fields: BTreeMap<String, String>,
    ) -> Self {
        Self {
            alias,
            protocol,
            fields,
        }
    }
}

/// A third party network user.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
pub struct User {
    /// A matrix user ID representing a third party user.
    pub user_id: OwnedUserId,

    /// The protocol ID that the third party user is a part of.
    pub protocol: String,

    /// Information used to identify this third party user.
    pub fields: BTreeMap<String, String>,
}

impl User {
    /// Creates a new `User` with the given user_id, protocol and fields.
    pub fn new(user_id: OwnedUserId, protocol: String, fields: BTreeMap<String, String>) -> Self {
        Self {
            user_id,
            protocol,
            fields,
        }
    }
}

/// The medium of a third party identifier.
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Medium {
    /// Email address identifier
    Email,

    /// Phone number identifier
    Msisdn,

    #[doc(hidden)]
    #[salvo(schema(value_type = String))]
    _Custom(PrivOwnedStr),
}

/// An identifier external to Matrix.
///
/// To create an instance of this type.
#[derive(ToSchema, Deserialize, Serialize, Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct ThirdPartyIdentifier {
    /// The third party identifier address.
    pub address: String,

    /// The medium of third party identifier.
    pub medium: Medium,

    /// The time when the identifier was validated by the identity server.
    pub validated_at: UnixMillis,

    /// The time when the homeserver associated the third party identifier with
    /// the user.
    pub added_at: UnixMillis,
}

// `GET /_matrix/client/*/thirdparty/protocols`
//
// Fetches the overall metadata about protocols supported by the homeserver.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartyprotocols
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/thirdparty/protocols",
//         1.1 => "/_matrix/client/v3/thirdparty/protocols",
//     }
// };

/// Response type for the `get_protocols` endpoint.

#[derive(ToSchema, Serialize, Default, Debug)]

pub struct ProtocolsResBody(
    /// Metadata about protocols supported by the homeserver.
    BTreeMap<String, Protocol>,
);

impl ProtocolsResBody {
    /// Creates a new `Response` with the given protocols.
    pub fn new(protocols: BTreeMap<String, Protocol>) -> Self {
        Self(protocols)
    }
}

// `GET /_matrix/client/*/thirdparty/protocol/{protocol}`
//
// Fetches the metadata from the homeserver about a particular third party
// protocol. `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartyprotocolprotocol
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/thirdparty/protocol/:protocol",
//         1.1 => "/_matrix/client/v3/thirdparty/protocol/:protocol",
//     }
// };
// Request type for the `get_protocol` endpoint.
// pub struct ProtocolReqBody {
//     /// The name of the protocol.
//     #[salvo(parameter(parameter_in = Path))]
//     pub protocol: String,
// }

/// Response type for the `get_protocol` endpoint.
#[derive(ToSchema, Serialize, Clone, Default, Debug)]
pub struct ProtocolResBody(
    /// Metadata about the protocol.
    pub Protocol,
);
impl ProtocolResBody {
    /// Creates a new `Response` with the given protocol.
    pub fn new(protocol: Protocol) -> Self {
        Self(protocol)
    }
}

// `GET /_matrix/client/*/thirdparty/location/{protocol}`
//
// Fetches third party locations for a protocol.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartylocationprotocol
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/thirdparty/location/:protocol",
//         1.1 => "/_matrix/client/v3/thirdparty/location/:protocol",
//     }
// };

// /// Request type for the `get_location_for_protocol` endpoint.
// pub struct LocationForProtocolReqBody {
//     /// The protocol used to communicate to the third party network.
//     #[salvo(parameter(parameter_in = Path))]
//     pub protocol: String,

//     /// One or more custom fields to help identify the third party location.
//     // The specification is incorrect for this parameter. See [matrix-spec#560](https://github.com/matrix-org/matrix-spec/issues/560).
//
//     pub fields: BTreeMap<String, String>,
// }

/// Response type for the `get_location_for_protocol` endpoint.
#[derive(ToSchema, Serialize, Default, Debug)]
pub struct LocationsResBody(
    /// List of matched third party locations.
    pub Vec<Location>,
);

impl LocationsResBody {
    /// Creates a new `Response` with the given locations.
    pub fn new(locations: Vec<Location>) -> Self {
        Self(locations)
    }
}

// `GET /_matrix/client/*/thirdparty/location`
//
// Retrieve an array of third party network locations from a Matrix room alias.
// `/v3/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartylocation
// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/client/r0/thirdparty/location",
//         1.1 => "/_matrix/client/v3/thirdparty/location",
//     }
// };

// /// Request type for the `get_location_for_room_alias` endpoint.
// pub struct LocationReqBody {
//     /// The Matrix room alias to look up.
//     #[salvo(parameter(parameter_in = Query))]
//     pub alias: OwnedRoomAliasId,
// }

// /// Response type for the `get_location_for_room_alias` endpoint.
// #[derive(ToSchema, Serialize, Debug)]
//
// pub struct LocationResBody {
//     /// List of matched third party locations.
//     pub locations: Vec<Location>,
// }
//
// impl LocationResBody {
//     /// Creates a new `Response` with the given locations.
//     pub fn new(locations: Vec<Location>) -> Self {
//         Self { locations }
//     }
// }

// /// `GET /_matrix/client/*/thirdparty/user/{protocol}`
// ///
// /// Fetches third party users for a protocol.
// /// `/v3/` ([spec])
// ///
// /// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartyuserprotocol
// /// const METADATA: Metadata = metadata! {
// ///     method: GET,
// ///     rate_limited: false,
// ///     authentication: AccessToken,
// ///     history: {
// ///         1.0 => "/_matrix/client/r0/thirdparty/user/:protocol",
// ///         1.1 => "/_matrix/client/v3/thirdparty/user/:protocol",
// ///     }
// /// };
//
// /// Request type for the `get_user_for_protocol` endpoint.

// pub struct UserForProtocolReqBody {
//     /// The protocol used to communicate to the third party network.
//     #[salvo(parameter(parameter_in = Path))]
//     pub protocol: String,

//     /// One or more custom fields that are passed to the AS to help identify the user.
//     // The specification is incorrect for this parameter. See [matrix-spec#560](https://github.com/matrix-org/matrix-spec/issues/560).
//
//     pub fields: BTreeMap<String, String>,
// }

/// `GET /_matrix/client/*/thirdparty/user`
///
/// Retrieve an array of third party users from a Matrix User ID.
/// `/v3/` ([spec])
///
/// [spec]: https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3thirdpartyuser
/// const METADATA: Metadata = metadata! {
///     method: GET,
///     rate_limited: false,
///     authentication: AccessToken,
///     history: {
///         1.0 => "/_matrix/client/r0/thirdparty/user",
///         1.1 => "/_matrix/client/v3/thirdparty/user",
///     }
/// };
///
/// Response type for the `get_user_for_protocol` endpoint.
#[derive(ToSchema, Serialize, Default, Debug)]
pub struct UsersResBody(
    /// List of matched third party users.
    pub Vec<User>,
);

impl UsersResBody {
    /// Creates a new `Response` with the given users.
    pub fn new(users: Vec<User>) -> Self {
        Self(users)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value as from_json_value, json, to_value as to_json_value};

    use super::{Medium, ThirdPartyIdentifier};
    use crate::UnixMillis;

    #[test]
    fn third_party_identifier_serde() {
        let third_party_id = ThirdPartyIdentifier {
            address: "monkey@banana.island".into(),
            medium: Medium::Email,
            validated_at: UnixMillis(1_535_176_800_000_u64.try_into().unwrap()),
            added_at: UnixMillis(1_535_336_848_756_u64.try_into().unwrap()),
        };

        let third_party_id_serialized = json!({
            "medium": "email",
            "address": "monkey@banana.island",
            "validated_at": 1_535_176_800_000_u64,
            "added_at": 1_535_336_848_756_u64
        });

        assert_eq!(
            to_json_value(third_party_id.clone()).unwrap(),
            third_party_id_serialized
        );
        assert_eq!(
            third_party_id,
            from_json_value(third_party_id_serialized).unwrap()
        );
    }
}
