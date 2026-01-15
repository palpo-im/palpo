//! Types for matrixRTC state events ([MSC3401]).
//!
//! This implements a newer/updated version of MSC3401.
//!
//! [MSC3401]: https://github.com/matrix-org/matrix-spec-proposals/pull/3401

mod focus;
mod member_data;
mod member_state_key;
use std::time::Duration;

use as_variant::as_variant;
pub use focus::*;
pub use member_data::*;
pub use member_state_key::*;
use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

use crate::events::{
    PossiblyRedactedStateEventContent, RedactContent, RedactedStateEventContent, StateEventType,
    StaticEventContent,
};
use crate::macros::EventContent;
use crate::room_version_rules::RedactionRules;
use crate::serde::StringEnum;
use crate::{OwnedDeviceId, PrivOwnedStr, UnixMillis};

/// The member state event for a matrixRTC session.
///
/// This is the object containing all the data related to a matrix users
/// participation in a matrixRTC session. It consists of memberships / sessions.
#[derive(ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize, EventContent)]
#[palpo_event(type = "org.matrix.msc3401.call.member", kind = State, state_key_type = CallMemberStateKey, custom_redacted, custom_possibly_redacted)]
#[serde(untagged)]
pub enum CallMemberEventContent {
    /// The legacy format for m.call.member events. (An array of memberships. The devices of one
    /// user.)
    LegacyContent(LegacyMembershipContent),
    /// Normal membership events. One event per membership. Multiple state keys will
    /// be used to describe multiple devices for one user.
    SessionContent(SessionMembershipData),
    /// An empty content means this user has been in a rtc session but is not anymore.
    Empty(EmptyMembershipData),
}

impl CallMemberEventContent {
    /// Creates a new [`CallMemberEventContent`] with [`LegacyMembershipData`].
    pub fn new_legacy(memberships: Vec<LegacyMembershipData>) -> Self {
        Self::LegacyContent(LegacyMembershipContent {
            memberships, //: memberships.into_iter().map(MembershipData::Legacy).collect(),
        })
    }

    /// Creates a new [`CallMemberEventContent`] with [`SessionMembershipData`].
    ///
    /// # Arguments
    /// * `application` - The application that is creating the membership.
    /// * `device_id` - The device ID of the member.
    /// * `focus_active` - The active focus state of the member.
    /// * `foci_preferred` - The preferred focus states of the member.
    /// * `created_ts` - The timestamp when this state event chain for memberships was created. when
    ///   updating the event the `created_ts` should be copied from the previous state. Set to
    ///   `None` if this is the initial join event for the session.
    /// * `expires` - The time after which the event is considered as expired. Defaults to 4 hours.
    pub fn new(
        application: Application,
        device_id: OwnedDeviceId,
        focus_active: ActiveFocus,
        foci_preferred: Vec<Focus>,
        created_ts: Option<UnixMillis>,
        expires: Option<Duration>,
    ) -> Self {
        Self::SessionContent(SessionMembershipData {
            application,
            device_id,
            focus_active,
            foci_preferred,
            created_ts,
            expires: expires.unwrap_or(Duration::from_secs(14_400)), // Default to 4 hours
        })
    }

    /// Creates a new Empty [`CallMemberEventContent`] representing a left membership.
    pub fn new_empty(leave_reason: Option<LeaveReason>) -> Self {
        Self::Empty(EmptyMembershipData { leave_reason })
    }

    /// All non expired memberships in this member event.
    ///
    /// In most cases you want tu use this method instead of the public
    /// memberships field. The memberships field will also include expired
    /// events.
    ///
    /// # Arguments
    ///
    /// * `origin_server_ts` - optionally the `origin_server_ts` can be passed as a fallback in case
    ///   the Membership does not contain `created_ts`. (`origin_server_ts` will be ignored if
    ///   `created_ts` is `Some`)
    pub fn active_memberships(
        &self,
        origin_server_ts: Option<UnixMillis>,
    ) -> Vec<MembershipData<'_>> {
        match self {
            CallMemberEventContent::LegacyContent(content) => content
                .memberships
                .iter()
                .map(MembershipData::Legacy)
                .filter(|m| !m.is_expired(origin_server_ts))
                .collect(),
            CallMemberEventContent::SessionContent(content) => {
                vec![MembershipData::Session(content)]
                    .into_iter()
                    .filter(|m| !m.is_expired(origin_server_ts))
                    .collect()
            }

            CallMemberEventContent::Empty(_) => Vec::new(),
        }
    }

    /// All the memberships for this event. Can only contain multiple elements in the case of legacy
    /// `m.call.member` state events.
    pub fn memberships(&self) -> Vec<MembershipData<'_>> {
        match self {
            CallMemberEventContent::LegacyContent(content) => content
                .memberships
                .iter()
                .map(MembershipData::Legacy)
                .collect(),
            CallMemberEventContent::SessionContent(content) => {
                [content].map(MembershipData::Session).to_vec()
            }
            CallMemberEventContent::Empty(_) => Vec::new(),
        }
    }

    /// Set the `created_ts` in this event.
    ///
    /// Each call member event contains the `origin_server_ts` and `content.create_ts`.
    /// `content.create_ts` is undefined for the initial event of a session (because the
    /// `origin_server_ts` is not known on the client).
    /// In the rust sdk we want to copy over the `origin_server_ts` of the event into the content.
    /// (This allows to use `MinimalStateEvents` and still be able to determine if a membership is
    /// expired)
    pub fn set_created_ts_if_none(&mut self, origin_server_ts: UnixMillis) {
        match self {
            CallMemberEventContent::LegacyContent(content) => {
                content
                    .memberships
                    .iter_mut()
                    .for_each(|m: &mut LegacyMembershipData| {
                        m.created_ts.get_or_insert(origin_server_ts);
                    });
            }
            CallMemberEventContent::SessionContent(m) => {
                m.created_ts.get_or_insert(origin_server_ts);
            }
            _ => (),
        }
    }
}

/// This describes the CallMember event if the user is not part of the current session.
#[derive(ToSchema, PartialEq, Clone, Serialize, Deserialize, Debug)]
pub struct EmptyMembershipData {
    /// An empty call member state event can optionally contain a leave reason.
    /// If it is `None` the user has left the call ordinarily. (Intentional hangup)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leave_reason: Option<LeaveReason>,
}

/// This is the optional value for an empty membership event content:
/// [`CallMemberEventContent::Empty`].
///
/// It is used when the user disconnected and a Future ([MSC4140](https://github.com/matrix-org/matrix-spec-proposals/pull/4140))
/// was used to update the membership after the client was not reachable anymore.
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all(prefix = "m.", rule = "snake_case"))]
pub enum LeaveReason {
    /// The user left the call by losing network connection or closing
    /// the client before it was able to send the leave event.
    LostConnection,
    #[doc(hidden)]
    _Custom(PrivOwnedStr),
}

impl RedactContent for CallMemberEventContent {
    type Redacted = RedactedCallMemberEventContent;

    fn redact(self, _rules: &RedactionRules) -> Self::Redacted {
        RedactedCallMemberEventContent {}
    }
}

/// The PossiblyRedacted version of [`CallMemberEventContent`].
///
/// Since [`CallMemberEventContent`] has the [`CallMemberEventContent::Empty`] state it already is
/// compatible with the redacted version of the state event content.
pub type PossiblyRedactedCallMemberEventContent = CallMemberEventContent;

impl PossiblyRedactedStateEventContent for PossiblyRedactedCallMemberEventContent {
    type StateKey = CallMemberStateKey;

    fn event_type(&self) -> StateEventType {
        StateEventType::CallMember
    }
}

/// The Redacted version of [`CallMemberEventContent`].
#[derive(ToSchema, Clone, Debug, Deserialize, Serialize)]
#[allow(clippy::exhaustive_structs)]
pub struct RedactedCallMemberEventContent {}

impl RedactedStateEventContent for RedactedCallMemberEventContent {
    type StateKey = CallMemberStateKey;

    fn event_type(&self) -> StateEventType {
        StateEventType::CallMember
    }
}

impl StaticEventContent for RedactedCallMemberEventContent {
    const TYPE: &'static str = CallMemberEventContent::TYPE;
    type IsPrefix = <CallMemberEventContent as StaticEventContent>::IsPrefix;
}

/// Legacy content with an array of memberships. See also: [`CallMemberEventContent`]
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacyMembershipContent {
    /// A list of all the memberships that user currently has in this room.
    ///
    /// There can be multiple ones in case the user participates with multiple devices or there
    /// are multiple RTC applications running.
    ///
    /// e.g. a call and a spacial experience.
    ///
    /// Important: This includes expired memberships.
    /// To retrieve a list including only valid memberships,
    /// see [`active_memberships`](CallMemberEventContent::active_memberships).
    memberships: Vec<LegacyMembershipData>,
}

/// A membership describes one of the sessions this user currently partakes.
///
/// The application defines the type of the session.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Membership {
    /// The type of the matrixRTC session the membership belongs to.
    ///
    /// e.g. call, spacial, document...
    #[serde(flatten)]
    pub application: Application,

    /// The device id of this membership.
    ///
    /// The same user can join with their phone/computer.
    pub device_id: String,

    /// The duration in milliseconds relative to the time this membership joined
    /// during which the membership is valid.
    ///
    /// The time a member has joined is defined as:
    /// `MIN(content.created_ts, event.origin_server_ts)`
    #[serde(with = "palpo_core::serde::duration::ms")]
    pub expires: Duration,

    /// Stores a copy of the `origin_server_ts` of the initial session event.
    ///
    /// If the membership is updated this field will be used to track to
    /// original `origin_server_ts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ts: Option<UnixMillis>,

    /// A list of the foci in use for this membership.
    pub foci_active: Vec<Focus>,

    /// The id of the membership.
    ///
    /// This is required to guarantee uniqueness of the event.
    /// Sending the same state event twice to synapse makes the HS drop the
    /// second one and return 200.
    #[serde(rename = "membershipID")]
    pub membership_id: String,
}

impl Membership {
    /// The application of the membership is "m.call" and the scope is "m.room".
    pub fn is_room_call(&self) -> bool {
        as_variant!(&self.application, Application::Call)
            .is_some_and(|call| call.scope == CallScope::Room)
    }

    /// The application of the membership is "m.call".
    pub fn is_call(&self) -> bool {
        as_variant!(&self.application, Application::Call).is_some()
    }

    /// Checks if the event is expired.
    ///
    /// Defaults to using `created_ts` of the `Membership`.
    /// If no `origin_server_ts` is provided and the event does not contain
    /// `created_ts` the event will be considered as not expired.
    /// In this case, a warning will be logged.
    ///
    /// # Arguments
    ///
    /// * `origin_server_ts` - a fallback if `created_ts` is not present
    pub fn is_expired(&self, origin_server_ts: Option<UnixMillis>) -> bool {
        let ev_created_ts = self.created_ts.or(origin_server_ts);

        if let Some(ev_created_ts) = ev_created_ts {
            let now = UnixMillis::now().to_system_time();
            let expire_ts = ev_created_ts.to_system_time().map(|t| t + self.expires);
            now > expire_ts
        } else {
            // This should not be reached since we only allow events that have copied over
            // the origin server ts. `set_created_ts_if_none`
            warn!(
                "Encountered a Call Member state event where the origin_ts (or origin_server_ts) could not be found.\
            It is treated as a non expired event but this might be wrong."
            );
            false
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use std::time::Duration;

//     use serde_json::json;

//     use super::{
//         Application, CallApplicationContent, CallMemberEventContent, CallScope, Membership,
//         focus::{ActiveFocus, ActiveLivekitFocus, Focus, LivekitFocus},
//         member_data::{
//             Application, CallApplicationContent, CallScope, LegacyMembershipData, MembershipData,
//         },
//     };
//     use crate::{
//         call::member::{EmptyMembershipData, FocusSelection, SessionMembershipData},
//         AnyStateEvent, StateEvent,
//     };
//     use crate::{owned_device_id,UnixMillis as TS};

// fn create_call_member_event_content() -> CallMemberEventContent {
//     CallMemberEventContent::new(vec![Membership {
//         application: Application::Call(CallApplicationContent {
//             call_id: "123456".to_owned(),
//             scope: CallScope::Room,
//         }),
//         device_id: "ABCDE".to_owned(),
//         expires: Duration::from_secs(3600),
//         foci_active: vec![Focus::Livekit(LivekitFocus {
//             alias: "1".to_owned(),
//             service_url: "https://livekit.com".to_owned(),
//         })],
//         membership_id: "0".to_owned(),
//         created_ts: None,
//     }])
// }

// #[test]
// fn serialize_call_member_event_content() {
//     let call_member_event = &json!({
//         "memberships": [
//             {
//                 "application": "m.call",
//                 "call_id": "123456",
//                 "scope": "m.room",
//                 "device_id": "ABCDE",
//                 "expires": 3_600_000,
//                 "foci_active": [
//                     {
//                         "livekit_alias": "1",
//                         "livekit_service_url": "https://livekit.com",
//                         "type": "livekit"
//                     }
//                 ],
//                 "membershipID": "0"
//             }
//         ]
//     });

//     assert_eq!(
//         call_member_event,
//         &serde_json::to_value(create_call_member_event_content()).unwrap()
//     );
// }

// #[test]
// fn deserialize_call_member_event_content() {
//     let call_member_ev = CallMemberEventContent::new(
//         Application::Call(CallApplicationContent {
//             call_id: "123456".to_owned(),
//             scope: CallScope::Room,
//         }),
//         owned_device_id!("THIS_DEVICE"),
//         ActiveFocus::Livekit(ActiveLivekitFocus {
//             focus_selection: FocusSelection::OldestMembership,
//         }),
//         vec![Focus::Livekit(LivekitFocus {
//             alias: "room1".to_owned(),
//             service_url: "https://livekit1.com".to_owned(),
//         })],
//         None,
//         None,
//     );

//     let call_member_ev_json = json!({
//         "application": "m.call",
//         "call_id": "123456",
//         "scope": "m.room",
//         "expires": 14_400_000, // Default to 4 hours
//         "device_id": "THIS_DEVICE",
//         "focus_active":{
//             "type": "livekit",
//             "focus_selection": "oldest_membership"
//         },
//         "foci_preferred": [
//             {
//                 "livekit_alias": "room1",
//                 "livekit_service_url": "https://livekit1.com",
//                 "type": "livekit"
//             }
//         ],
//     });

//     let ev_content: CallMemberEventContent =
//         serde_json::from_value(call_member_ev_json).unwrap();
//     assert_eq!(
//         serde_json::to_string(&ev_content).unwrap(),
//         serde_json::to_string(&call_member_ev).unwrap()
//     );
//     let empty = CallMemberEventContent::Empty(EmptyMembershipData { leave_reason: None });
//     assert_eq!(
//         serde_json::to_string(&json!({})).unwrap(),
//         serde_json::to_string(&empty).unwrap()
//     );
// }

// fn timestamps() -> (TS, TS, TS) {
//     let now = TS::now();
//     let one_second_ago = now
//         .to_system_time()
//         .unwrap()
//         .checked_sub(Duration::from_secs(1))
//         .unwrap();
//     let two_hours_ago = now
//         .to_system_time()
//         .unwrap()
//         .checked_sub(Duration::from_secs(60 * 60 * 2))
//         .unwrap();
//     (
//         now,
//         TS::from_system_time(one_second_ago).unwrap(),
//         TS::from_system_time(two_hours_ago).unwrap(),
//     )
// }

// #[test]
// fn legacy_memberships_do_expire() {
//     let content_legacy = create_call_member_legacy_event_content();
//     let (now, one_second_ago, two_hours_ago) = timestamps();

//     assert_eq!(
//         content_legacy.active_memberships(Some(one_second_ago)),
//         content_legacy.memberships()
//     );
//     assert_eq!(
//         content_legacy.active_memberships(Some(now)),
//         content_legacy.memberships()
//     );
//     assert_eq!(
//         content_legacy.active_memberships(Some(two_hours_ago)),
//         (vec![] as Vec<MembershipData<'_>>)
//     );
// }

// #[test]
// fn session_membership_does_expire() {
//     let content = create_call_member_event_content();
//     let (now, one_second_ago, two_hours_ago) = timestamps();

//     assert_eq!(content.active_memberships(Some(now)), content.memberships());
//     assert_eq!(
//         content.active_memberships(Some(one_second_ago)),
//         content.memberships()
//     );
//     assert_eq!(
//         content.active_memberships(Some(two_hours_ago)),
//         (vec![] as Vec<MembershipData<'_>>)
//     );
// }

// #[test]
// fn set_created_ts() {
//     let mut content_now = create_call_member_event_content();
//     let mut content_two_hours_ago = create_call_member_event_content();
//     let mut content_one_second_ago = create_call_member_event_content();
//     let (now, one_second_ago, two_hours_ago) = timestamps();

//     content_now.set_created_ts_if_none(now);
//     content_one_second_ago.set_created_ts_if_none(one_second_ago);
//     content_two_hours_ago.set_created_ts_if_none(two_hours_ago);
//     assert_eq!(
//         content_now.active_memberships(None),
//         content_now.memberships()
//     );

//     assert_eq!(
//         content_two_hours_ago.active_memberships(None),
//         vec![] as Vec<&Membership>
//     );
//     assert_eq!(
//         content_one_second_ago.active_memberships(None),
//         content_one_second_ago.memberships()
//     );

//     // created_ts should not be overwritten.
//     content_two_hours_ago.set_created_ts_if_none(one_second_ago);
//     // There still should be no active membership.
//     assert_eq!(
//         content_two_hours_ago.active_memberships(None),
//         vec![] as Vec<MembershipData<'_>>
//     );
// }
// }
