//! Endpoints for third party lookups

// `GET /_matrix/app/*/thirdparty/location/{protocol}`
//
// Retrieve a list of Matrix portal rooms that lead to the matched third party
// location. `/v1/` ([spec])
//
// [spec]: https://spec.matrix.org/latest/application-service-api/#get_matrixappv1thirdpartylocationprotocol
use std::collections::BTreeMap;

use salvo::extract::{Extractible, Metadata};
use salvo::http::ParseError;
use salvo::oapi::{Components, EndpointArgRegister, Operation, Parameter, ParameterIn, Parameters};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};

use crate::third_party::{Location, Protocol, User};

// const METADATA: Metadata = metadata! {
//     method: GET,
//     rate_limited: false,
//     authentication: AccessToken,
//     history: {
//         1.0 => "/_matrix/app/v1/thirdparty/location/:protocol",
//     }
// };

/// Request type for the `get_location_for_protocol` endpoint.
#[derive(Deserialize, Debug)]
pub struct ForProtocolReqArgs {
    /// The protocol used to communicate to the third party network.
    pub protocol: String,

    /// One or more custom fields to help identify the third party location.
    // The specification is incorrect for this parameter. See [matrix-spec#560](https://github.com/matrix-org/matrix-spec/issues/560).
    pub fields: BTreeMap<String, String>,
}

impl<'ex> Extractible<'ex> for ForProtocolReqArgs {
    fn metadata() -> &'static Metadata {
        static METADATA: Metadata = Metadata::new("ForProtocolReqArgs");
        &METADATA
    }

    #[allow(refining_impl_trait)]
    async fn extract(req: &'ex mut Request, _depot: &'ex mut Depot) -> Result<Self, ParseError> {
        // Custom fields are separate query parameters, not a JSON-encoded `fields` value.
        let protocol = req.param("protocol").ok_or(ParseError::NotExist)?;
        let mut fields: BTreeMap<String, String> = req.parse_queries()?;
        fields.remove("access_token");
        Ok(Self { protocol, fields })
    }
}

impl<'ex> ToParameters<'ex> for ForProtocolReqArgs {
    fn to_parameters(components: &mut Components) -> Parameters {
        // The protocol defines the query field names at runtime, so only the
        // fixed path parameter can be registered as a named OpenAPI parameter.
        Parameters::new().parameter(
            Parameter::new("protocol")
                .location(ParameterIn::Path)
                .required(true)
                .schema(String::to_schema(components)),
        )
    }
}

impl EndpointArgRegister for ForProtocolReqArgs {
    fn register(components: &mut Components, operation: &mut Operation, _arg: &str) {
        operation
            .parameters
            .append(&mut Self::to_parameters(components));
    }
}
impl ForProtocolReqArgs {
    /// Creates a new `Request` with the given protocol.
    pub fn new(protocol: String) -> Self {
        Self {
            protocol,
            fields: BTreeMap::new(),
        }
    }
}
// /// Request type for the `get_location_for_room_alias` endpoint.
// #[request]
// pub struct ForRoomAliasReqArgs {
//     /// The Matrix room alias to look up.
//     pub alias: OwnedRoomAliasId,
// }

// impl ForRoomAliasReqBody {
//     /// Creates a new `Request` with the given room alias id.
//     pub fn new(alias: OwnedRoomAliasId) -> Self {
//         Self { alias }
//     }
// }

/// Response type for the `get_location_for_protocol` endpoint.
#[derive(ToSchema, Serialize, Debug)]
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

/// Response type for the `get_user_for_protocol` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct ProtocolResBody {
    /// Metadata about the protocol.
    pub protocol: Protocol,
}
impl ProtocolResBody {
    /// Creates a new `Response` with the given protocol.
    pub fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }
}

/// Response type for the `get_location_for_protocol` endpoint.
#[derive(ToSchema, Serialize, Debug)]
pub struct UsersResBody {
    /// List of matched third party users.
    pub users: Vec<User>,
}

impl UsersResBody {
    /// Creates a new `Response` with the given users.
    pub fn new(users: Vec<User>) -> Self {
        Self { users }
    }
}
