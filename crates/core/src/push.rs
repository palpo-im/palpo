//! Common types for the [push notifications module][push].
//!
//! [push]: https://spec.matrix.org/latest/client-server-api/#push-notifications
//!
//! ## Understanding the types of this module
//!
//! Push rules are grouped in `RuleSet`s, and are grouped in five kinds (for
//! more details about the different kind of rules, see the `Ruleset`
//! documentation, or the specification). These five kinds are, by order of
//! priority:
//!
//! - override rules
//! - content rules
//! - room rules
//! - sender rules
//! - underride rules

mod action;
mod condition;
mod conditional_push_rule;
mod error;
mod iter;
mod patterned_push_rule;
mod predefined;
pub mod push_gateway;
mod push_rule;
mod pusher;
mod ruleset;
mod simple_push_rule;
use std::hash::Hash;

pub use conditional_push_rule::*;
pub use error::*;
use indexmap::{Equivalent, IndexSet};
pub use patterned_push_rule::*;
pub use push_rule::{RuleKind, *};
pub use pusher::*;
pub use ruleset::Ruleset;
use salvo::oapi::ToSchema;
pub use simple_push_rule::*;

pub use self::action::{Action, Tweak};
#[cfg(feature = "unstable-msc3932")]
pub use self::condition::RoomVersionFeature;
pub use self::condition::{
    _CustomPushCondition, ComparisonOperator, FlattenedJson, FlattenedJsonValue, PushCondition,
    PushConditionPowerLevelsCtx, PushConditionRoomCtx, RoomMemberCountIs, ScalarJsonValue,
};
pub use self::iter::{AnyPushRule, AnyPushRuleRef, RulesetIntoIter, RulesetIter};
pub use self::predefined::{
    PredefinedContentRuleId, PredefinedOverrideRuleId, PredefinedRuleId, PredefinedUnderrideRuleId,
};
use crate::PrivOwnedStr;
use crate::serde::StringEnum;

/// A special format that the homeserver should use when sending notifications
/// to a Push Gateway. Currently, only `event_id_only` is supported, see the
/// [Push Gateway API][spec].
///
/// [spec]: https://spec.matrix.org/latest/push-gateway-api/#homeserver-behaviour
#[doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/doc/string_enum.md"))]
#[derive(ToSchema, Clone, StringEnum)]
#[palpo_enum(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PushFormat {
    /// Require the homeserver to only send a reduced set of fields in the push.
    EventIdOnly,

    #[doc(hidden)]
    #[salvo(schema(skip))]
    _Custom(PrivOwnedStr),
}

/// Insert the rule in the given indexset and move it to the given position.
pub fn insert_and_move_rule<T>(
    set: &mut IndexSet<T>,
    rule: T,
    default_position: usize,
    after: Option<&str>,
    before: Option<&str>,
) -> Result<(), InsertPushRuleError>
where
    T: Hash + Eq,
    str: Equivalent<T>,
{
    let (from, replaced) = set.replace_full(rule);

    let mut to = default_position;

    if let Some(rule_id) = after {
        let idx = set
            .get_index_of(rule_id)
            .ok_or(InsertPushRuleError::UnknownRuleId)?;
        to = idx + 1;
    }
    if let Some(rule_id) = before {
        let idx = set
            .get_index_of(rule_id)
            .ok_or(InsertPushRuleError::UnknownRuleId)?;

        if idx < to {
            return Err(InsertPushRuleError::BeforeHigherThanAfter);
        }

        to = idx;
    }

    // Only move the item if it's new or if it was positioned.
    if replaced.is_none() || after.is_some() || before.is_some() {
        set.move_index(from, to);
    }

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use std::collections::BTreeMap;

//     use assert_matches2::assert_matches;
//     use serde_json::{from_value as from_json_value, json, to_value as
// to_json_value};

//     use super::{
//         action::{Action, Tweak},
//         condition::{PushCondition, PushConditionPowerLevelsCtx,
// PushConditionRoomCtx, RoomMemberCountIs},         AnyPushRule,
// ConditionalPushRule, PatternedPushRule, Ruleset, SimplePushRule,     };
//     use crate::{
//         owned_room_id, owned_user_id,
//         power_levels::NotificationPowerLevels,
//         push::{PredefinedContentRuleId, PredefinedOverrideRuleId},
//         serde::RawJson,
//         user_id, JsonValue, RawJsonValue,
//     };

//     fn example_ruleset() -> Ruleset {
//         let mut set = Ruleset::new();

//         set.override_.insert(ConditionalPushRule {
//             conditions: vec![PushCondition::EventMatch {
//                 key: "type".into(),
//                 pattern: "m.call.invite".into(),
//             }],
//             actions: vec![Action::Notify,
// Action::SetTweak(Tweak::Highlight(true))],             rule_id:
// ".m.rule.call".into(),             enabled: true,
//             default: true,
//         });

//         set
//     }

//     fn power_levels() -> PushConditionPowerLevelsCtx {
//         PushConditionPowerLevelsCtx {
//             users: BTreeMap::new(),
//             users_default: 50,
//             notifications: NotificationPowerLevels { room: 50 },
//         }
//     }

//     #[test]
//     fn iter() {
//         let mut set = example_ruleset();

//         let added = set.override_.insert(ConditionalPushRule {
//             conditions: vec![PushCondition::EventMatch {
//                 key: "room_id".into(),
//                 pattern: "!roomid:matrix.org".into(),
//             }],
//             actions: vec![],
//             rule_id: "!roomid:matrix.org".into(),
//             enabled: true,
//             default: false,
//         });
//         assert!(added);

//         let added = set.override_.insert(ConditionalPushRule {
//             conditions: vec![],
//             actions: vec![],
//             rule_id: ".m.rule.suppress_notices".into(),
//             enabled: false,
//             default: true,
//         });
//         assert!(added);

//         let mut iter = set.into_iter();

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Override(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, ".m.rule.call");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Override(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, "!roomid:matrix.org");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Override(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, ".m.rule.suppress_notices");

//         assert_matches!(iter.next(), None);
//     }

//     #[test]
//     fn serialize_conditional_push_rule() {
//         let rule = ConditionalPushRule {
//             actions: vec![Action::Notify,
// Action::SetTweak(Tweak::Highlight(true))],             default: true,
//             enabled: true,
//             rule_id: ".m.rule.call".into(),
//             conditions: vec![
//                 PushCondition::EventMatch {
//                     key: "type".into(),
//                     pattern: "m.call.invite".into(),
//                 },
//                 PushCondition::ContainsDisplayName,
//                 PushCondition::RoomMemberCount {
//                     is: RoomMemberCountIs::gt(2),
//                 },
//                 PushCondition::SenderNotificationPermission { key:
// "room".into() },             ],
//         };

//         let rule_value: JsonValue = to_json_value(rule).unwrap();
//         assert_eq!(
//             rule_value,
//             json!({
//                 "conditions": [
//                     {
//                         "kind": "event_match",
//                         "key": "type",
//                         "pattern": "m.call.invite"
//                     },
//                     {
//                         "kind": "contains_display_name"
//                     },
//                     {
//                         "kind": "room_member_count",
//                         "is": ">2"
//                     },
//                     {
//                         "kind": "sender_notification_permission",
//                         "key": "room"
//                     }
//                 ],
//                 "actions": [
//                     "notify",
//                     {
//                         "set_tweak": "highlight"
//                     }
//                 ],
//                 "rule_id": ".m.rule.call",
//                 "default": true,
//                 "enabled": true
//             })
//         );
//     }

//     #[test]
//     fn serialize_simple_push_rule() {
//         let rule = SimplePushRule {
//             actions: vec![Action::Notify],
//             default: false,
//             enabled: false,
//             rule_id: owned_room_id!("!roomid:server.name"),
//         };

//         let rule_value: JsonValue = to_json_value(rule).unwrap();
//         assert_eq!(
//             rule_value,
//             json!({
//                 "actions": [
//                     "notify"
//                 ],
//                 "rule_id": "!roomid:server.name",
//                 "default": false,
//                 "enabled": false
//             })
//         );
//     }

//     #[test]
//     fn serialize_patterned_push_rule() {
//         let rule = PatternedPushRule {
//             actions: vec![
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound("default".into())),
//                 Action::SetTweak(Tweak::Custom {
//                     name: "dance".into(),
//                     value: RawJsonValue::from_string("true".into()).unwrap(),
//                 }),
//             ],
//             default: true,
//             enabled: true,
//             pattern: "user_id".into(),
//             rule_id: ".m.rule.contains_user_name".into(),
//         };

//         let rule_value: JsonValue = to_json_value(rule).unwrap();
//         assert_eq!(
//             rule_value,
//             json!({
//                 "actions": [
//                     "notify",
//                     {
//                         "set_tweak": "sound",
//                         "value": "default"
//                     },
//                     {
//                         "set_tweak": "dance",
//                         "value": true
//                     }
//                 ],
//                 "pattern": "user_id",
//                 "rule_id": ".m.rule.contains_user_name",
//                 "default": true,
//                 "enabled": true
//             })
//         );
//     }

//     #[test]
//     fn serialize_ruleset() {
//         let mut set = example_ruleset();

//         set.override_.insert(ConditionalPushRule {
//             conditions: vec![
//                 PushCondition::RoomMemberCount {
//                     is: RoomMemberCountIs::from(2),
//                 },
//                 PushCondition::EventMatch {
//                     key: "type".into(),
//                     pattern: "m.room.message".into(),
//                 },
//             ],
//             actions: vec![
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound("default".into())),
//                 Action::SetTweak(Tweak::Highlight(false)),
//             ],
//             rule_id: ".m.rule.room_one_to_one".into(),
//             enabled: true,
//             default: true,
//         });
//         set.content.insert(PatternedPushRule {
//             actions: vec![
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound("default".into())),
//                 Action::SetTweak(Tweak::Highlight(true)),
//             ],
//             rule_id: ".m.rule.contains_user_name".into(),
//             pattern: "user_id".into(),
//             enabled: true,
//             default: true,
//         });

//         let set_value: JsonValue = to_json_value(set).unwrap();
//         assert_eq!(
//             set_value,
//             json!({
//                 "override": [
//                     {
//                         "actions": [
//                             "notify",
//                             {
//                                 "set_tweak": "highlight",
//                             },
//                         ],
//                         "conditions": [
//                             {
//                                 "kind": "event_match",
//                                 "key": "type",
//                                 "pattern": "m.call.invite"
//                             },
//                         ],
//                         "rule_id": ".m.rule.call",
//                         "default": true,
//                         "enabled": true,
//                     },
//                     {
//                         "conditions": [
//                             {
//                                 "kind": "room_member_count",
//                                 "is": "2"
//                             },
//                             {
//                                 "kind": "event_match",
//                                 "key": "type",
//                                 "pattern": "m.room.message"
//                             }
//                         ],
//                         "actions": [
//                             "notify",
//                             {
//                                 "set_tweak": "sound",
//                                 "value": "default"
//                             },
//                             {
//                                 "set_tweak": "highlight",
//                                 "value": false
//                             }
//                         ],
//                         "rule_id": ".m.rule.room_one_to_one",
//                         "default": true,
//                         "enabled": true
//                     },
//                 ],
//                 "content": [
//                     {
//                         "actions": [
//                             "notify",
//                             {
//                                 "set_tweak": "sound",
//                                 "value": "default"
//                             },
//                             {
//                                 "set_tweak": "highlight"
//                             }
//                         ],
//                         "pattern": "user_id",
//                         "rule_id": ".m.rule.contains_user_name",
//                         "default": true,
//                         "enabled": true
//                     }
//                 ],
//             })
//         );
//     }

//     #[test]
//     fn deserialize_patterned_push_rule() {
//         let rule = from_json_value::<PatternedPushRule>(json!({
//             "actions": [
//                 "notify",
//                 {
//                     "set_tweak": "sound",
//                     "value": "default"
//                 },
//                 {
//                     "set_tweak": "highlight",
//                     "value": true
//                 }
//             ],
//             "pattern": "user_id",
//             "rule_id": ".m.rule.contains_user_name",
//             "default": true,
//             "enabled": true
//         }))
//         .unwrap();
//         assert!(rule.default);
//         assert!(rule.enabled);
//         assert_eq!(rule.pattern, "user_id");
//         assert_eq!(rule.rule_id, ".m.rule.contains_user_name");

//         let mut iter = rule.actions.iter();
//         assert_matches!(iter.next(), Some(Action::Notify));
//         assert_matches!(iter.next(),
// Some(Action::SetTweak(Tweak::Sound(sound))));         assert_eq!(sound,
// "default");         assert_matches!(iter.next(),
// Some(Action::SetTweak(Tweak::Highlight(true))));         assert_matches!
// (iter.next(), None);     }

//     #[test]
//     fn deserialize_ruleset() {
//         let set: Ruleset = from_json_value(json!({
//             "override": [
//                 {
//                     "actions": [],
//                     "conditions": [],
//                     "rule_id": "!roomid:server.name",
//                     "default": false,
//                     "enabled": true
//                 },
//                 {
//                     "actions": [],
//                     "conditions": [],
//                     "rule_id": ".m.rule.call",
//                     "default": true,
//                     "enabled": true
//                 },
//             ],
//             "underride": [
//                 {
//                     "actions": [],
//                     "conditions": [],
//                     "rule_id": ".m.rule.room_one_to_one",
//                     "default": true,
//                     "enabled": true
//                 },
//             ],
//             "room": [
//                 {
//                     "actions": [],
//                     "rule_id": "!roomid:server.name",
//                     "default": false,
//                     "enabled": false
//                 }
//             ],
//             "sender": [],
//             "content": [
//                 {
//                     "actions": [],
//                     "pattern": "user_id",
//                     "rule_id": ".m.rule.contains_user_name",
//                     "default": true,
//                     "enabled": true
//                 },
//                 {
//                     "actions": [],
//                     "pattern": "palpo",
//                     "rule_id": "palpo",
//                     "default": false,
//                     "enabled": true
//                 }
//             ]
//         }))
//         .unwrap();

//         let mut iter = set.into_iter();

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Override(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, "!roomid:server.name");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Override(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, ".m.rule.call");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Content(PatternedPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, ".m.rule.contains_user_name");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Content(PatternedPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, "palpo");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(rule_opt.unwrap(), AnyPushRule::Room(SimplePushRule {
// rule_id, .. }));         assert_eq!(rule_id, "!roomid:server.name");

//         let rule_opt = iter.next();
//         assert!(rule_opt.is_some());
//         assert_matches!(
//             rule_opt.unwrap(),
//             AnyPushRule::Underride(ConditionalPushRule { rule_id, .. })
//         );
//         assert_eq!(rule_id, ".m.rule.room_one_to_one");

//         assert_matches!(iter.next(), None);
//     }

//     #[test]
//     fn default_ruleset_applies() {
//         let set =
// Ruleset::server_default(user_id!("@jolly_jumper:server.name"));

//         let context_one_to_one = &PushConditionRoomCtx {
//             room_id: owned_room_id!("!dm:server.name"),
//             member_count: u2,
//             user_id: owned_user_id!("@jj:server.name"),
//             user_display_name: "Jolly Jumper".into(),
//             power_levels: Some(power_levels()),

//             supported_features: Default::default(),
//         };

//         let context_public_room = &PushConditionRoomCtx {
//             room_id: owned_room_id!("!far_west:server.name"),
//             member_count: u100,
//             user_id: owned_user_id!("@jj:server.name"),
//             user_display_name: "Jolly Jumper".into(),
//             power_levels: Some(power_levels()),

//             supported_features: Default::default(),
//         };

//         let message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "type": "m.room.message"
//             }"#,
//         )
//         .unwrap();

//         assert_matches!(
//             set.get_actions(&message, context_one_to_one),
//             [
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound(_)),
//                 Action::SetTweak(Tweak::Highlight(false))
//             ]
//         );
//         assert_matches!(
//             set.get_actions(&message, context_public_room),
//             [Action::Notify, Action::SetTweak(Tweak::Highlight(false))]
//         );

//         let user_name = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "type": "m.room.message",
//                 "content": {
//                     "body": "Hi jolly_jumper!"
//                 }
//             }"#,
//         )
//         .unwrap();

//         assert_matches!(
//             set.get_actions(&user_name, context_one_to_one),
//             [
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound(_)),
//                 Action::SetTweak(Tweak::Highlight(true)),
//             ]
//         );
//         assert_matches!(
//             set.get_actions(&user_name, context_public_room),
//             [
//                 Action::Notify,
//                 Action::SetTweak(Tweak::Sound(_)),
//                 Action::SetTweak(Tweak::Highlight(true)),
//             ]
//         );

//         let notice = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "type": "m.room.message",
//                 "content": {
//                     "msgtype": "m.notice"
//                 }
//             }"#,
//         )
//         .unwrap();
//         assert_matches!(set.get_actions(&notice, context_one_to_one), []);

//         let at_room = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "type": "m.room.message",
//                 "sender": "@rantanplan:server.name",
//                 "content": {
//                     "body": "@room Attention please!",
//                     "msgtype": "m.text"
//                 }
//             }"#,
//         )
//         .unwrap();

//         assert_matches!(
//             set.get_actions(&at_room, context_public_room),
//             [Action::Notify, Action::SetTweak(Tweak::Highlight(true)),]
//         );

//         let empty =
// serde_json::from_str::<RawJson<JsonValue>>(r#"{}"#).unwrap();
//         assert_matches!(set.get_actions(&empty, context_one_to_one), []);
//     }

//     #[test]
//     fn custom_ruleset_applies() {
//         let context_one_to_one = &PushConditionRoomCtx {
//             room_id: owned_room_id!("!dm:server.name"),
//             member_count: u2,
//             user_id: owned_user_id!("@jj:server.name"),
//             user_display_name: "Jolly Jumper".into(),
//             power_levels: Some(power_levels()),

//             supported_features: Default::default(),
//         };

//         let message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "sender": "@rantanplan:server.name",
//                 "type": "m.room.message",
//                 "content": {
//                     "msgtype": "m.text",
//                     "body": "Great joke!"
//                 }
//             }"#,
//         )
//         .unwrap();

//         let mut set = Ruleset::new();
//         let disabled = ConditionalPushRule {
//             actions: vec![Action::Notify],
//             default: false,
//             enabled: false,
//             rule_id: "disabled".into(),
//             conditions: vec![PushCondition::RoomMemberCount {
//                 is: RoomMemberCountIs::from(2),
//             }],
//         };
//         set.underride.insert(disabled);

//         let test_set = set.clone();
//         assert_matches!(test_set.get_actions(&message, context_one_to_one),
// []);

//         let no_conditions = ConditionalPushRule {
//             actions: vec![Action::SetTweak(Tweak::Highlight(true))],
//             default: false,
//             enabled: true,
//             rule_id: "no.conditions".into(),
//             conditions: vec![],
//         };
//         set.underride.insert(no_conditions);

//         let test_set = set.clone();
//         assert_matches!(
//             test_set.get_actions(&message, context_one_to_one),
//             [Action::SetTweak(Tweak::Highlight(true))]
//         );

//         let sender = SimplePushRule {
//             actions: vec![Action::Notify],
//             default: false,
//             enabled: true,
//             rule_id: owned_user_id!("@rantanplan:server.name"),
//         };
//         set.sender.insert(sender);

//         let test_set = set.clone();
//         assert_matches!(test_set.get_actions(&message, context_one_to_one),
// [Action::Notify]);

//         let room = SimplePushRule {
//             actions: vec![Action::SetTweak(Tweak::Highlight(true))],
//             default: false,
//             enabled: true,
//             rule_id: owned_room_id!("!dm:server.name"),
//         };
//         set.room.insert(room);

//         let test_set = set.clone();
//         assert_matches!(
//             test_set.get_actions(&message, context_one_to_one),
//             [Action::SetTweak(Tweak::Highlight(true))]
//         );

//         let content = PatternedPushRule {
//             actions: vec![Action::SetTweak(Tweak::Sound("content".into()))],
//             default: false,
//             enabled: true,
//             rule_id: "content".into(),
//             pattern: "joke".into(),
//         };
//         set.content.insert(content);

//         let test_set = set.clone();
//         assert_matches!(
//             test_set.get_actions(&message, context_one_to_one),
//             [Action::SetTweak(Tweak::Sound(sound))]
//         );
//         assert_eq!(sound, "content");

//         let three_conditions = ConditionalPushRule {
//             actions: vec![Action::SetTweak(Tweak::Sound("three".into()))],
//             default: false,
//             enabled: true,
//             rule_id: "three.conditions".into(),
//             conditions: vec![
//                 PushCondition::RoomMemberCount {
//                     is: RoomMemberCountIs::from(2),
//                 },
//                 PushCondition::ContainsDisplayName,
//                 PushCondition::EventMatch {
//                     key: "room_id".into(),
//                     pattern: "!dm:server.name".into(),
//                 },
//             ],
//         };
//         set.override_.insert(three_conditions);

//         assert_matches!(
//             set.get_actions(&message, context_one_to_one),
//             [Action::SetTweak(Tweak::Sound(sound))]
//         );
//         assert_eq!(sound, "content");

//         let new_message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "sender": "@rantanplan:server.name",
//                 "type": "m.room.message",
//                 "content": {
//                     "msgtype": "m.text",
//                     "body": "Tell me another one, Jolly Jumper!"
//                 }
//             }"#,
//         )
//         .unwrap();

//         assert_matches!(
//             set.get_actions(&new_message, context_one_to_one),
//             [Action::SetTweak(Tweak::Sound(sound))]
//         );
//         assert_eq!(sound, "three");
//     }
//     #[test]
//     fn intentional_mentions_apply() {
//         let set =
// Ruleset::server_default(user_id!("@jolly_jumper:server.name"));

//         let context = &PushConditionRoomCtx {
//             room_id: owned_room_id!("!far_west:server.name"),
//             member_count: u100,
//             user_id: owned_user_id!("@jj:server.name"),
//             user_display_name: "Jolly Jumper".into(),
//             power_levels: Some(power_levels()),

//             supported_features: Default::default(),
//         };

//         let message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "content": {
//                     "body": "Hey jolly_jumper!",
//                     "m.mentions": {
//                         "user_ids": ["@jolly_jumper:server.name"]
//                     }
//                 },
//                 "sender": "@admin:server.name",
//                 "type": "m.room.message"
//             }"#,
//         )
//         .unwrap();

//         assert_eq!(
//             set.get_match(&message, context).unwrap().rule_id(),
//             PredefinedOverrideRuleId::IsUserMention.as_ref()
//         );

//         let message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "content": {
//                     "body": "Listen room!",
//                     "m.mentions": {
//                         "room": true
//                     }
//                 },
//                 "sender": "@admin:server.name",
//                 "type": "m.room.message"
//             }"#,
//         )
//         .unwrap();

//         assert_eq!(
//             set.get_match(&message, context).unwrap().rule_id(),
//             PredefinedOverrideRuleId::IsRoomMention.as_ref()
//         );
//     }

//     #[test]
//     fn invite_for_me_applies() {
//         let set =
// Ruleset::server_default(user_id!("@jolly_jumper:server.name"));

//         let context = &PushConditionRoomCtx {
//             room_id: owned_room_id!("!far_west:server.name"),
//             member_count: u100,
//             user_id: owned_user_id!("@jj:server.name"),
//             user_display_name: "Jolly Jumper".into(),
//             // `invite_state` usually doesn't include the power levels.
//             power_levels: None,

//             supported_features: Default::default(),
//         };

//         let message = serde_json::from_str::<RawJson<JsonValue>>(
//             r#"{
//                 "content": {
//                     "membership": "invite"
//                 },
//                 "state_key": "@jolly_jumper:server.name",
//                 "sender": "@admin:server.name",
//                 "type": "m.room.member"
//             }"#,
//         )
//         .unwrap();

//         assert_eq!(
//             set.get_match(&message, context).unwrap().rule_id(),
//             PredefinedOverrideRuleId::InviteForMe.as_ref()
//         );
//     }
// }
