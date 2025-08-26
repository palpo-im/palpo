//! Types for MatrixRTC `m.call.member` state event content data ([MSC3401])
//!
//! [MSC3401]: https://github.com/matrix-org/matrix-spec-proposals/pull/3401

use std::time::Duration;

use as_variant::as_variant;
use serde::{Deserialize, Serialize};
use tracing::warn;
use salvo::oapi::ToSchema;

use super::focus::{ActiveFocus, ActiveLivekitFocus, Focus};
use crate::PrivOwnedStr;
use crate::{DeviceId, UnixMillis, OwnedDeviceId};
use crate::macros::StringEnum;

/// The data object that contains the information for one membership.
///
/// It can be a legacy or a normal MatrixRTC Session membership.
///
/// The legacy format contains time information to compute if it is expired or not.
/// SessionMembershipData does not have the concept of timestamp based expiration anymore.
/// The state event will reliably be set to empty when the user disconnects.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum MembershipData<'a> {
    /// The legacy format (using an array of memberships for each device -> one event per user)
    Legacy(&'a LegacyMembershipData),
    /// One event per device. `SessionMembershipData` contains all the information required to
    /// represent the current membership state of one device.
    Session(&'a SessionMembershipData),
}

impl MembershipData<'_> {
    /// The application this RTC membership participates in (the session type, can be `m.call`...)
    pub fn application(&self) -> &Application {
        match self {
            MembershipData::Legacy(data) => &data.application,
            MembershipData::Session(data) => &data.application,
        }
    }

    /// The device id of this membership.
    pub fn device_id(&self) -> &DeviceId {
        match self {
            MembershipData::Legacy(data) => &data.device_id,
            MembershipData::Session(data) => &data.device_id,
        }
    }

    /// The active focus is a FocusType specific object that describes how this user
    /// is currently connected.
    ///
    /// It can use the foci_preferred list to choose one of the available (preferred)
    /// foci or specific information on how to connect to this user.
    ///
    /// Every user needs to converge to use the same focus_active type.
    pub fn focus_active(&self) -> &ActiveFocus {
        match self {
            MembershipData::Legacy(_) => &ActiveFocus::Livekit(ActiveLivekitFocus {
                focus_selection: super::focus::FocusSelection::OldestMembership,
            }),
            MembershipData::Session(data) => &data.focus_active,
        }
    }

    /// The list of available/preferred options this user provides to connect to the call.
    pub fn foci_preferred(&self) -> &Vec<Focus> {
        match self {
            MembershipData::Legacy(data) => &data.foci_active,
            MembershipData::Session(data) => &data.foci_preferred,
        }
    }

    /// The application of the membership is "m.call" and the scope is "m.room".
    pub fn is_room_call(&self) -> bool {
        as_variant!(self.application(), Application::Call)
            .is_some_and(|call| call.scope == CallScope::Room)
    }

    /// The application of the membership is "m.call".
    pub fn is_call(&self) -> bool {
        as_variant!(self.application(), Application::Call).is_some()
    }

    /// Gets the created_ts of the event.
    ///
    /// This is the `origin_server_ts` for session data.
    /// For legacy events this can either be the origin server ts or a copy from the
    /// `origin_server_ts` since we expect legacy events to get updated (when a new device
    /// joins/leaves).
    pub fn created_ts(&self) -> Option<UnixMillis> {
        match self {
            MembershipData::Legacy(data) => data.created_ts,
            MembershipData::Session(data) => data.created_ts,
        }
    }

    /// Checks if the event is expired.
    ///
    /// Defaults to using `created_ts` of the [`MembershipData`].
    /// If no `origin_server_ts` is provided and the event does not contain `created_ts`
    /// the event will be considered as not expired.
    /// In this case, a warning will be logged.
    ///
    /// This method needs to be called periodically to check if the event is still valid.
    ///
    /// # Arguments
    ///
    /// * `origin_server_ts` - a fallback if [`MembershipData::created_ts`] is not present
    pub fn is_expired(&self, origin_server_ts: Option<UnixMillis>) -> bool {
        if let Some(expire_ts) = self.expires_ts(origin_server_ts) {
            UnixMillis::now() > expire_ts
        } else {
            // This should not be reached since we only allow events that have copied over
            // the origin server ts. `set_created_ts_if_none`
            warn!("Encountered a Call Member state event where the expire_ts could not be constructed.");
            false
        }
    }

    /// The unix timestamp at which the event will expire.
    /// This allows to determine at what time the return value of
    /// [`MembershipData::is_expired`] will change.
    ///
    /// Defaults to using `created_ts` of the [`MembershipData`].
    /// If no `origin_server_ts` is provided and the event does not contain `created_ts`
    /// the event will be considered as not expired.
    /// In this case, a warning will be logged.
    ///
    /// # Arguments
    ///
    /// * `origin_server_ts` - a fallback if [`MembershipData::created_ts`] is not present
    pub fn expires_ts(
        &self,
        origin_server_ts: Option<UnixMillis>,
    ) -> Option<UnixMillis> {
        let expires = match &self {
            MembershipData::Legacy(data) => data.expires,
            MembershipData::Session(data) => data.expires,
        };
        let ev_created_ts = self.created_ts().or(origin_server_ts)?.to_system_time();
        ev_created_ts.and_then(|t| UnixMillis::from_system_time(t + expires))
    }
}

/// A membership describes one of the sessions this user currently partakes.
///
/// The application defines the type of the session.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacyMembershipData {
    /// The type of the MatrixRTC session the membership belongs to.
    ///
    /// e.g. call, spacial, document...
    #[serde(flatten)]
    pub application: Application,

    /// The device id of this membership.
    ///
    /// The same user can join with their phone/computer.
    pub device_id: OwnedDeviceId,

    /// The duration in milliseconds relative to the time this membership joined
    /// during which the membership is valid.
    ///
    /// The time a member has joined is defined as:
    /// `MIN(content.created_ts, event.origin_server_ts)`
    #[serde(with = "crate::serde::duration::ms")]
    pub expires: Duration,

    /// Stores a copy of the `origin_server_ts` of the initial session event.
    ///
    /// If the membership is updated this field will be used to track the
    /// original `origin_server_ts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ts: Option<UnixMillis>,

    /// A list of the foci in use for this membership.
    pub foci_active: Vec<Focus>,

    /// The id of the membership.
    ///
    /// This is required to guarantee uniqueness of the event.
    /// Sending the same state event twice to synapse makes the HS drop the second one and return
    /// 200.
    #[serde(rename = "membershipID")]
    pub membership_id: String,
}


/// Stores all the information for a MatrixRTC membership. (one for each device)
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionMembershipData {
    /// The type of the MatrixRTC session the membership belongs to.
    ///
    /// e.g. call, spacial, document...
    #[serde(flatten)]
    pub application: Application,

    /// The device id of this membership.
    ///
    /// The same user can join with their phone/computer.
    pub device_id: OwnedDeviceId,

    /// A list of the foci that this membership proposes to use.
    pub foci_preferred: Vec<Focus>,

    /// Data required to determine the currently used focus by this member.
    pub focus_active: ActiveFocus,

    /// Stores a copy of the `origin_server_ts` of the initial session event.
    ///
    /// If the membership is updated this field will be used to track the
    /// original `origin_server_ts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ts: Option<UnixMillis>,

    /// The duration in milliseconds relative to the time this membership joined
    /// during which the membership is valid.
    ///
    /// The time a member has joined is defined as:
    /// `MIN(content.created_ts, event.origin_server_ts)`
    #[serde(with = "crate::serde::duration::ms")]
    pub expires: Duration,
}
