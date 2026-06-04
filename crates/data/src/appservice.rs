use std::fmt;

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::{Deserialize, Serialize};

use crate::core::appservice::Registration;
use crate::core::serde::JsonValue;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Insertable, Serialize, Deserialize, Clone)]
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

    /// Whether this appservice is administratively disabled.
    ///
    /// Disabled appservices are loaded but not returned by the enabled list, so
    /// they neither receive events nor authenticate requests.
    #[serde(default)]
    pub disabled: bool,
}

// Custom Debug implementation to prevent leaking as_token and hs_token
impl fmt::Debug for DbRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbRegistration")
            .field("id", &self.id)
            .field("url", &self.url)
            .field("as_token", &"[REDACTED]")
            .field("hs_token", &"[REDACTED]")
            .field("sender_localpart", &self.sender_localpart)
            .field("namespaces", &self.namespaces)
            .field("rate_limited", &self.rate_limited)
            .field("protocols", &self.protocols)
            .field("receive_ephemeral", &self.receive_ephemeral)
            .field("device_management", &self.device_management)
            .field("disabled", &self.disabled)
            .finish()
    }
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
            disabled: false,
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
            disabled: _,
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

/// Insert a new appservice registration.
pub async fn insert_registration(registration: &DbRegistration) -> DataResult<()> {
    diesel::insert_into(appservice_registrations::table)
        .values(registration)
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Delete an appservice registration by id.
pub async fn delete_registration(id: &str) -> DataResult<()> {
    diesel::delete(appservice_registrations::table.find(id))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Set the `disabled` flag on an appservice. Returns true if a row was updated.
pub async fn set_disabled(id: &str, disabled: bool) -> DataResult<bool> {
    let affected = diesel::update(appservice_registrations::table.find(id))
        .set(appservice_registrations::disabled.eq(disabled))
        .execute(&mut connect().await?)
        .await?;
    Ok(affected > 0)
}

/// Load every registration, including administratively disabled ones.
pub async fn all_registrations() -> DataResult<Vec<DbRegistration>> {
    appservice_registrations::table
        .load::<DbRegistration>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Load only enabled (not administratively disabled) registrations.
pub async fn enabled_registrations() -> DataResult<Vec<DbRegistration>> {
    appservice_registrations::table
        .filter(appservice_registrations::disabled.eq(false))
        .load::<DbRegistration>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Fetch a single registration by id.
pub async fn find_registration(id: &str) -> DataResult<Option<DbRegistration>> {
    appservice_registrations::table
        .find(id)
        .first::<DbRegistration>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}
