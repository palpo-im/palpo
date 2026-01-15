use palpo_core::events::push_rules::PushRulesEventContent;
use salvo::oapi::extract::JsonBody;
use salvo::prelude::*;

use crate::core::client::push::{
    ConditionalReqBody, PatternedReqBody, RuleActionsResBody, RuleEnabledResBody, RuleResBody,
    RulesResBody, SetRuleActionsReqBody, SetRuleEnabledReqBody, SetRuleReqArgs, SimpleReqBody,
};
use crate::core::events::GlobalAccountDataEventType;
use crate::core::push::{
    InsertPushRuleError, NewConditionalPushRule, NewPatternedPushRule, NewPushRule,
    NewSimplePushRule, RemovePushRuleError, RuleKind, RuleScope, ScopeKindRuleReqArgs,
};
use crate::{
    DepotExt, EmptyResult, JsonResult, MatrixError, OptionalExtension, empty_ok, hoops, json_ok,
};

pub fn authed_router() -> Router {
    Router::with_path("pushrules")
        .get(list_rules)
        .push(Router::with_path("global").get(global))
        .push(
            Router::with_path("{scope}/{kind}/{rule_id}")
                .get(get_rule)
                .delete(delete_rule)
                .push(
                    Router::with_path("actions")
                        .get(get_actions)
                        .put(set_actions),
                )
                .push(
                    Router::with_path("enabled")
                        .get(get_enabled)
                        .put(set_enabled),
                ),
        )
        .push(
            Router::with_hoop(hoops::limit_rate)
                .push(Router::with_path("{scope}/{kind}/{rule_id}").put(set_rule)),
        )
}

#[endpoint]
async fn global() -> EmptyResult {
    // TODO: todo
    empty_ok()
}

/// #GET /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}
/// Retrieves a single specified push rule for this user.
#[endpoint]
fn get_rule(args: ScopeKindRuleReqArgs, depot: &mut Depot) -> JsonResult<RuleResBody> {
    let authed = depot.authed_info()?;

    let user_data_content = crate::data::user::get_global_data::<PushRulesEventContent>(
        authed.user_id(),
        &GlobalAccountDataEventType::PushRules.to_string(),
    )?
    .ok_or(MatrixError::not_found("push rule event not found."))?;

    let rule = user_data_content
        .global
        .get(args.kind.clone(), &args.rule_id)
        .map(Into::into);

    if let Some(rule) = rule {
        json_ok(RuleResBody { rule })
    } else {
        Err(MatrixError::not_found("push rule not found").into())
    }
}

/// #PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}
/// Creates a single specified push rule for this user.
#[endpoint]
async fn set_rule(args: SetRuleReqArgs, req: &mut Request, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;
    let payload = req.payload().await?;
    let new_rule: NewPushRule = match &args.kind {
        RuleKind::Override => {
            let ConditionalReqBody {
                actions,
                conditions,
            } = serde_json::from_slice(payload)?;
            NewPushRule::Override(NewConditionalPushRule::new(
                args.rule_id.clone(),
                conditions,
                actions,
            ))
        }
        RuleKind::Underride => {
            let ConditionalReqBody {
                actions,
                conditions,
            } = serde_json::from_slice(payload)?;
            NewPushRule::Underride(NewConditionalPushRule::new(
                args.rule_id.clone(),
                conditions,
                actions,
            ))
        }
        RuleKind::Sender => {
            let SimpleReqBody { actions } = serde_json::from_slice(payload)?;
            let rule_id = args.rule_id.clone().try_into()?;
            NewPushRule::Sender(NewSimplePushRule::new(rule_id, actions))
        }
        RuleKind::Room => {
            let SimpleReqBody { actions } = serde_json::from_slice(payload)?;
            let rule_id = args.rule_id.clone().try_into()?;
            NewPushRule::Room(NewSimplePushRule::new(rule_id, actions))
        }
        RuleKind::Content => {
            let PatternedReqBody { actions, pattern } = serde_json::from_slice(payload)?;
            NewPushRule::Content(NewPatternedPushRule::new(
                args.rule_id.clone(),
                pattern,
                actions,
            ))
        }
        _ => {
            return Err(MatrixError::invalid_param("invalid rule kind").into());
        }
    };

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("scopes other than 'global' are not supported").into(),
        );
    }

    let mut user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )
    .unwrap_or_default();

    if let Err(error) =
        user_data_content
            .global
            .insert(new_rule, args.after.as_deref(), args.before.as_deref())
    {
        let err = match error {
            InsertPushRuleError::ServerDefaultRuleId => MatrixError::invalid_param(
                "rule ids starting with a dot are reserved for server-default rules",
            ),
            InsertPushRuleError::InvalidRuleId => {
                MatrixError::invalid_param("rule ids containing invalid characters")
            }
            InsertPushRuleError::RelativeToServerDefaultRule => MatrixError::invalid_param(
                "can't place a push rule relatively to a server-default rule",
            ),
            InsertPushRuleError::UnknownRuleId => {
                MatrixError::not_found("The before or after rule could not be found.")
            }
            InsertPushRuleError::BeforeHigherThanAfter => MatrixError::invalid_param(
                "the before rule has a higher priority than the after rule",
            ),
            _ => MatrixError::invalid_param("invalid data"),
        };

        return Err(err.into());
    }

    crate::data::user::set_data(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(user_data_content)?,
    )?;

    empty_ok()
}

/// #DELETE /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}
/// Deletes a single specified push rule for this user.
#[endpoint]
async fn delete_rule(args: ScopeKindRuleReqArgs, depot: &mut Depot) -> EmptyResult {
    let authed = depot.authed_info()?;

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("Scopes other than 'global' are not supported.").into(),
        );
    }

    let mut user_data_content = crate::data::user::get_global_data::<PushRulesEventContent>(
        authed.user_id(),
        &GlobalAccountDataEventType::PushRules.to_string(),
    )?
    .ok_or(MatrixError::not_found("PushRules event not found."))?;

    if let Err(error) = user_data_content
        .global
        .remove(args.kind.clone(), &args.rule_id)
    {
        let err = match error {
            RemovePushRuleError::ServerDefault => {
                MatrixError::invalid_param("cannot delete a server-default push rule")
            }
            RemovePushRuleError::NotFound => MatrixError::not_found("Push rule not found."),
            _ => MatrixError::invalid_param("invalid data"),
        };

        return Err(err.into());
    }

    crate::data::user::set_data(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(user_data_content)?,
    )?;
    empty_ok()
}

/// #GET /_matrix/client/r0/pushrules
/// Retrieves the push rules event for this user.
#[endpoint]
async fn list_rules(depot: &mut Depot) -> JsonResult<RulesResBody> {
    let authed = depot.authed_info()?;

    let user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )
    .unwrap_or_default();

    json_ok(RulesResBody {
        global: user_data_content.global,
    })
}

/// #GET /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}/actions
/// Gets the actions of a single specified push rule for this user.
#[endpoint]
async fn get_actions(
    args: ScopeKindRuleReqArgs,
    depot: &mut Depot,
) -> JsonResult<RuleActionsResBody> {
    let authed = depot.authed_info()?;

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("scopes other than 'global' are not supported").into(),
        );
    }

    let user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )
    .unwrap_or_default();

    let actions = user_data_content
        .global
        .get(args.kind.clone(), &args.rule_id)
        .map(|rule| rule.actions().to_owned())
        .ok_or(MatrixError::not_found("push rule not found"))?;

    json_ok(RuleActionsResBody { actions })
}

/// #PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}/actions
/// Sets the actions of a single specified push rule for this user.
#[endpoint]
fn set_actions(
    args: ScopeKindRuleReqArgs,
    body: JsonBody<SetRuleActionsReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("Scopes other than 'global' are not supported.").into(),
        );
    }

    let mut user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )
    .map_err(|_| MatrixError::not_found("push rules event not found"))?;

    if user_data_content
        .global
        .set_actions(args.kind.clone(), &args.rule_id, body.actions.clone())
        .is_err()
    {
        return Err(MatrixError::not_found("push rule not found").into());
    }

    crate::data::user::set_data(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(user_data_content).expect("to json value always works"),
    )?;

    empty_ok()
}

/// #GET /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}/enabled
/// Gets the enabled status of a single specified push rule for this user.
#[endpoint]
fn get_enabled(args: ScopeKindRuleReqArgs, depot: &mut Depot) -> JsonResult<RuleEnabledResBody> {
    let authed = depot.authed_info()?;

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("scopes other than 'global' are not supported").into(),
        );
    }

    let user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )?;

    let enabled = user_data_content
        .global
        .get(args.kind.clone(), &args.rule_id)
        .map(|r| r.enabled())
        .ok_or(MatrixError::not_found("Push rule not found."))?;

    json_ok(RuleEnabledResBody { enabled })
}

/// #PUT /_matrix/client/r0/pushrules/{scope}/{kind}/{rule_id}/enabled
/// Sets the enabled status of a single specified push rule for this user.
#[endpoint]
fn set_enabled(
    args: ScopeKindRuleReqArgs,
    body: JsonBody<SetRuleEnabledReqBody>,
    depot: &mut Depot,
) -> EmptyResult {
    let authed = depot.authed_info()?;

    if args.scope != RuleScope::Global {
        return Err(
            MatrixError::invalid_param("scopes other than 'global' are not supported").into(),
        );
    }

    let mut user_data_content = crate::data::user::get_data::<PushRulesEventContent>(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
    )?;

    if user_data_content
        .global
        .set_enabled(args.kind.clone(), &args.rule_id, body.enabled)
        .is_err()
    {
        return Err(MatrixError::not_found("push rule not found").into());
    }

    crate::data::user::set_data(
        authed.user_id(),
        None,
        &GlobalAccountDataEventType::PushRules.to_string(),
        serde_json::to_value(user_data_content)?,
    )?;

    empty_ok()
}
