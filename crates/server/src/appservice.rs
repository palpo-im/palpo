use std::collections::BTreeMap;
use std::time::Duration;

use diesel::prelude::*;
use regex::RegexSet;
use serde::{Deserialize, Serialize};

use crate::core::appservice::{Namespace, Registration};
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::data::connect;
use crate::data::schema::*;
use crate::{AppError, AppResult, sending};

/// Compiled regular expressions for a namespace.
#[derive(Clone, Debug)]
pub struct NamespaceRegex {
    pub exclusive: Option<RegexSet>,
    pub non_exclusive: Option<RegexSet>,
}

impl NamespaceRegex {
    /// Checks if this namespace has rights to a namespace
    pub fn is_match(&self, heystack: &str) -> bool {
        if self.is_exclusive_match(heystack) {
            return true;
        }

        if let Some(non_exclusive) = &self.non_exclusive
            && non_exclusive.is_match(heystack)
        {
            return true;
        }
        false
    }

    /// Checks if this namespace has exlusive rights to a namespace
    pub fn is_exclusive_match(&self, heystack: &str) -> bool {
        if let Some(exclusive) = &self.exclusive
            && exclusive.is_match(heystack)
        {
            return true;
        }
        false
    }
}

impl TryFrom<Vec<Namespace>> for NamespaceRegex {
    fn try_from(value: Vec<Namespace>) -> Result<Self, regex::Error> {
        let mut exclusive = vec![];
        let mut non_exclusive = vec![];

        for namespace in value {
            if namespace.exclusive {
                exclusive.push(namespace.regex);
            } else {
                non_exclusive.push(namespace.regex);
            }
        }

        Ok(NamespaceRegex {
            exclusive: if exclusive.is_empty() {
                None
            } else {
                Some(RegexSet::new(exclusive)?)
            },
            non_exclusive: if non_exclusive.is_empty() {
                None
            } else {
                Some(RegexSet::new(non_exclusive)?)
            },
        })
    }

    type Error = regex::Error;
}

/// Appservice registration combined with its compiled regular expressions.
#[derive(Clone, Debug)]
pub struct RegistrationInfo {
    pub registration: Registration,
    pub users: NamespaceRegex,
    pub aliases: NamespaceRegex,
    pub rooms: NamespaceRegex,
}

impl RegistrationInfo {
    /// Checks if a given user ID matches either the users namespace or the localpart specified in
    /// the appservice registration
    pub fn is_user_match(&self, user_id: &UserId) -> bool {
        self.users.is_match(user_id.as_str())
            || self.registration.sender_localpart == user_id.localpart()
    }

    /// Checks if a given user ID exclusively matches either the users namespace or the localpart
    /// specified in the appservice registration
    pub fn is_exclusive_user_match(&self, user_id: &UserId) -> bool {
        self.users.is_exclusive_match(user_id.as_str())
            || self.registration.sender_localpart == user_id.localpart()
    }
}
impl AsRef<Registration> for RegistrationInfo {
    fn as_ref(&self) -> &Registration {
        &self.registration
    }
}

impl TryFrom<Registration> for RegistrationInfo {
    type Error = regex::Error;

    fn try_from(value: Registration) -> Result<RegistrationInfo, Self::Error> {
        Ok(RegistrationInfo {
            users: value.namespaces.users.clone().try_into()?,
            aliases: value.namespaces.aliases.clone().try_into()?,
            rooms: value.namespaces.rooms.clone().try_into()?,
            registration: value,
        })
    }
}
impl TryFrom<DbRegistration> for RegistrationInfo {
    type Error = AppError;
    fn try_from(value: DbRegistration) -> Result<RegistrationInfo, Self::Error> {
        let value: Registration = value.try_into()?;
        Ok(RegistrationInfo {
            users: value.namespaces.users.clone().try_into()?,
            aliases: value.namespaces.aliases.clone().try_into()?,
            rooms: value.namespaces.rooms.clone().try_into()?,
            registration: value,
        })
    }
}

#[derive(Identifiable, Queryable, Insertable, Serialize, Deserialize, Clone, Debug)]
#[diesel(table_name = appservice_registrations)]
pub struct DbRegistration {
    /// A unique, user - defined ID of the application service which will never change.
    pub id: String,

    /// The URL for the application service.
    ///
    /// Optionally set to `null` if no traffic is required.
    pub url: Option<String>,

    /// A unique token for application services to use to authenticate requests to HomeServers.
    pub as_token: String,

    /// A unique token for HomeServers to use to authenticate requests to application services.
    pub hs_token: String,

    /// The localpart of the user associated with the application service.
    pub sender_localpart: String,

    /// A list of users, aliases and rooms namespaces that the application service controls.
    pub namespaces: JsonValue,

    /// Whether requests from masqueraded users are rate-limited.
    ///
    /// The sender is excluded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limited: Option<bool>,

    /// The external protocols which the application service provides (e.g. IRC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocols: Option<JsonValue>,

    /// Whether the application service wants to receive ephemeral data.
    ///
    /// Defaults to `false`.
    pub receive_ephemeral: bool,

    /// Whether the application service wants to do device management, as part of MSC4190.
    ///
    /// Defaults to `false`
    #[serde(default, rename = "io.element.msc4190")]
    pub device_management: bool,
}

impl From<Registration> for DbRegistration {
    fn from(value: Registration) -> Self {
        let Registration {
            id,
            url,
            as_token,
            hs_token,
            sender_localpart,
            namespaces,
            rate_limited,
            protocols,
            receive_ephemeral,
            device_management,
        } = value;
        Self {
            id,
            url,
            as_token,
            hs_token,
            sender_localpart,
            namespaces: serde_json::to_value(namespaces).unwrap_or_default(),
            rate_limited,
            protocols: protocols
                .map(|protocols| serde_json::to_value(protocols).unwrap_or_default()),
            receive_ephemeral,
            device_management,
        }
    }
}
impl TryFrom<DbRegistration> for Registration {
    type Error = serde_json::Error;

    fn try_from(value: DbRegistration) -> Result<Self, Self::Error> {
        let DbRegistration {
            id,
            url,
            as_token,
            hs_token,
            sender_localpart,
            namespaces,
            rate_limited,
            protocols,
            receive_ephemeral,
            device_management,
        } = value;
        let protocols = if let Some(protocols) = protocols {
            serde_json::from_value(protocols)?
        } else {
            None
        };
        Ok(Self {
            id,
            url,
            as_token,
            hs_token,
            sender_localpart,
            namespaces: serde_json::from_value(namespaces)?,
            rate_limited,
            protocols,
            receive_ephemeral,
            device_management,
        })
    }
}

/// Registers an appservice and returns the ID to the caller
pub fn register_appservice(registration: Registration) -> AppResult<String> {
    let db_registration: DbRegistration = registration.into();
    diesel::insert_into(appservice_registrations::table)
        .values(&db_registration)
        .execute(&mut connect()?)?;
    Ok(db_registration.id)
}

/// Remove an appservice registration
///
/// # Arguments
///
/// * `service_name` - the name you send to register the service previously
pub fn unregister_appservice(id: &str) -> AppResult<()> {
    diesel::delete(appservice_registrations::table.find(id)).execute(&mut connect()?)?;
    Ok(())
}

pub fn get_registration(id: &str) -> AppResult<Option<Registration>> {
    if let Some(registration) = appservice_registrations::table
        .find(id)
        .first::<DbRegistration>(&mut connect()?)
        .optional()?
    {
        Ok(Some(registration.try_into()?))
    } else {
        Ok(None)
    }
}
pub async fn find_from_token(token: &str) -> AppResult<Option<RegistrationInfo>> {
    Ok(all()?
        .values()
        .find(|info| info.registration.as_token == token)
        .cloned())
}

// Checks if a given user id matches any exclusive appservice regex
pub fn is_exclusive_user_id(user_id: &UserId) -> AppResult<bool> {
    for info in all()?.values() {
        if info.is_exclusive_user_match(user_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room alias matches any exclusive appservice regex
pub async fn is_exclusive_alias(alias: &RoomAliasId) -> AppResult<bool> {
    for info in all()?.values() {
        if info.aliases.is_exclusive_match(alias.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room id matches any exclusive appservice regex
pub async fn is_exclusive_room_id(room_id: &RoomId) -> AppResult<bool> {
    for info in all()?.values() {
        if info.rooms.is_exclusive_match(room_id.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn all() -> AppResult<BTreeMap<String, RegistrationInfo>> {
    let registrations = appservice_registrations::table.load::<DbRegistration>(&mut connect()?)?;
    Ok(registrations
        .into_iter()
        .filter_map(|db_registration| {
            let info: RegistrationInfo = match db_registration.try_into() {
                Ok(registration) => registration,
                Err(e) => {
                    warn!("Failed to parse appservice registration: {}", e);
                    return None;
                }
            };
            Some((info.registration.id.clone(), info))
        })
        .collect())
}

/// Sends a request to an appservice
///
/// Only returns None if there is no url specified in the appservice registration file
#[tracing::instrument(skip(request))]
pub(crate) async fn send_request(
    registration: Registration,
    mut request: reqwest::Request,
) -> AppResult<reqwest::Response> {
    let destination = match registration.url {
        Some(url) => url,
        None => {
            return Err(AppError::public("destination is none"));
        }
    };

    let hs_token = registration.hs_token.as_str();

    // let mut http_request = request
    //     .try_into_http_request::<BytesMut>(
    //         &destination,
    //         SendAccessToken::IfRequired(hs_token),
    //         &[MatrixVersion::V1_0],
    //     )
    //     .unwrap()
    //     .map(|body| body.freeze());

    request
        .url_mut()
        .query_pairs_mut()
        .append_pair("access_token", hs_token);

    // let mut reqwest_request = reqwest::Request::try_from(http_request)?;

    *request.timeout_mut() = Some(Duration::from_secs(30));

    let url = request.url().clone();
    let response = match sending::default_client().execute(request).await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                "Could not send request to appservice {:?} at {}: {}",
                registration.id, destination, e
            );
            return Err(e.into());
        }
    };

    // reqwest::Response -> http::Response conversion
    let status = response.status();
    // std::mem::swap(
    //     response.headers_mut(),
    //     http_response_builder
    //         .headers_mut()
    //         .expect("http::response::Builder is usable"),
    // );

    // let body = response.bytes().await.unwrap_or_else(|e| {
    //     warn!("server error: {}", e);
    //     Vec::new().into()
    // }); // TODO: handle timeout

    if status != 200 {
        warn!(
            "Appservice returned bad response {} {}\n{}",
            destination, status, url,
        );
    }

    // let response = T::IncomingResponse::try_from_http_response(
    //     http_response_builder
    //         .body(body)
    //         .expect("reqwest body is valid http body"),
    // );

    Ok(response)
}
