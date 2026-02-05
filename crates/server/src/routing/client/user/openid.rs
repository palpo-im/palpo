use std::time::Duration;

use diesel::prelude::*;
use salvo::oapi::extract::PathParam;
use salvo::prelude::*;

use crate::core::authentication::TokenType;
use crate::core::client::user::RequstOpenidTokenResBody;
use crate::core::identifiers::*;
use crate::core::UnixMillis;
use crate::data::connect;
use crate::data::schema::*;
use crate::{config, utils, AuthArgs, DepotExt, JsonResult, MatrixError, json_ok};

const OPENID_TOKEN_LENGTH: usize = 64;

/// Request an OpenID 1.0 token to verify identity with a third party.
///
/// The access_token generated is only valid for the OpenID Connect userinfo
/// endpoint at `/_matrix/federation/v1/openid/userinfo`.
#[endpoint]
pub(super) async fn request_token(
    _aa: AuthArgs,
    user_id: PathParam<OwnedUserId>,
    depot: &mut Depot,
) -> JsonResult<RequstOpenidTokenResBody> {
    let authed = depot.authed_info()?;

    // Verify the user is requesting a token for themselves
    let user_id = user_id.into_inner();
    if authed.user_id() != &user_id {
        return Err(MatrixError::forbidden(
            "Cannot request OpenID token for another user.",
            None,
        )
        .into());
    }

    let conf = config::get();
    let expires_in_secs = conf.openid_token_ttl;
    let expires_at = UnixMillis::now().get() as i64 + (expires_in_secs as i64 * 1000);

    let access_token = utils::random_string(OPENID_TOKEN_LENGTH);

    // Store the token in the database
    diesel::insert_into(user_openid_tokens::table)
        .values((
            user_openid_tokens::user_id.eq(authed.user_id()),
            user_openid_tokens::token.eq(&access_token),
            user_openid_tokens::expires_at.eq(expires_at),
        ))
        .execute(&mut connect()?)?;

    json_ok(RequstOpenidTokenResBody::new(
        access_token,
        TokenType::Bearer,
        conf.server_name.clone(),
        Duration::from_secs(expires_in_secs),
    ))
}
