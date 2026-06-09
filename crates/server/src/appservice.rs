use std::collections::BTreeMap;
use std::time::Duration;

use regex::RegexSet;
use subtle::ConstantTimeEq;

use crate::core::appservice::{Namespace, Registration};
use crate::core::identifiers::*;
pub use crate::data::appservice::DbRegistration;
use crate::{AppError, AppResult, data, sending};

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

/// Registers an appservice and returns the ID to the caller
pub async fn register_appservice(registration: Registration) -> AppResult<String> {
    let db_registration: DbRegistration = registration.into();
    data::appservice::insert_registration(&db_registration).await?;
    Ok(db_registration.id)
}

/// Remove an appservice registration
///
/// # Arguments
///
/// * `service_name` - the name you send to register the service previously
pub async fn unregister_appservice(id: &str) -> AppResult<()> {
    data::appservice::delete_registration(id).await?;
    Ok(())
}

/// Set the `disabled` flag on an appservice. Returns true if a row was updated.
pub async fn set_appservice_disabled(id: &str, disabled: bool) -> AppResult<bool> {
    Ok(data::appservice::set_disabled(id, disabled).await?)
}

/// List all registrations in the database, including disabled ones.
pub async fn list_all_registrations() -> AppResult<Vec<(DbRegistration, bool)>> {
    let regs = data::appservice::all_registrations().await?;
    Ok(regs
        .into_iter()
        .map(|r| {
            let disabled = r.disabled;
            (r, disabled)
        })
        .collect())
}

pub async fn get_registration(id: &str) -> AppResult<Option<Registration>> {
    if let Some(registration) = data::appservice::find_registration(id).await? {
        Ok(Some(registration.try_into()?))
    } else {
        Ok(None)
    }
}
pub async fn find_from_token(token: &str) -> AppResult<Option<RegistrationInfo>> {
    // Constant-time comparison so we don't leak `as_token` bytes via
    // response timing. The same pattern is used in `hoops/auth.rs`.
    Ok(all()
        .await?
        .values()
        .find(|info| {
            info.registration
                .as_token
                .as_bytes()
                .ct_eq(token.as_bytes())
                .into()
        })
        .cloned())
}

// Checks if a given user id matches any exclusive appservice regex
pub async fn is_exclusive_user_id(user_id: &UserId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.is_exclusive_user_match(user_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room alias matches any exclusive appservice regex
pub async fn is_exclusive_alias(alias: &RoomAliasId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.aliases.is_exclusive_match(alias.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

// Checks if a given room id matches any exclusive appservice regex
pub async fn is_exclusive_room_id(room_id: &RoomId) -> AppResult<bool> {
    for info in all().await?.values() {
        if info.rooms.is_exclusive_match(room_id.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn all() -> AppResult<BTreeMap<String, RegistrationInfo>> {
    let registrations = data::appservice::enabled_registrations().await?;
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
    let client = sending::default_client();
    let response = match reqwest::Client::execute(&client, request).await {
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
        let redacted_url = redacted_access_token_url(&url);
        warn!(
            "Appservice returned bad response {} {}\n{}",
            destination, status, redacted_url,
        );
    }

    // let response = T::IncomingResponse::try_from_http_response(
    //     http_response_builder
    //         .body(body)
    //         .expect("reqwest body is valid http body"),
    // );

    Ok(response)
}

fn redacted_access_token_url(url: &url::Url) -> url::Url {
    let mut redacted = url.clone();
    let Some(_) = redacted.query() else {
        return redacted;
    };

    let query_pairs = redacted
        .query_pairs()
        .map(|(key, value)| {
            let value = if key == "access_token" {
                "REDACTED".to_owned()
            } else {
                value.into_owned()
            };
            (key.into_owned(), value)
        })
        .collect::<Vec<_>>();

    redacted.set_query(None);
    redacted.query_pairs_mut().extend_pairs(
        query_pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    redacted
}

#[cfg(test)]
mod tests {
    use super::redacted_access_token_url;

    #[test]
    fn redacts_access_token_query_parameter() {
        let url = url::Url::parse(
            "https://appservice.example/_matrix/app/v1/transactions/1?foo=bar&access_token=secret",
        )
        .unwrap();

        let redacted = redacted_access_token_url(&url);
        let redacted = redacted.as_str();

        assert!(redacted.contains("foo=bar"));
        assert!(redacted.contains("access_token=REDACTED"));
        assert!(!redacted.contains("secret"));
    }
}
