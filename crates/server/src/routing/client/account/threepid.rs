//! `POST /_matrix/client/*/account/3pid/add`
//!
//! Add contact information to a user's account
//!
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3account3pidadd
//!
//! Registration can associate a locally verified email address. Identity-server
//! publishing and post-registration contact changes remain unsupported.

use salvo::prelude::*;

use crate::core::client::account::threepid::ThreepidsResBody;
use crate::{AuthArgs, EmptyResult, JsonResult, MatrixError, json_ok};
use crate::exts::DepotExt;

pub fn authed_router() -> Router {
    Router::with_path("3pid")
        .get(get)
        .push(Router::with_path("add").post(add))
        .push(Router::with_path("bind").post(bind))
        .push(Router::with_path("unbind").post(unbind))
        .push(Router::with_path("delete").post(delete))
}

/// #GET _matrix/client/v3/account/3pid
/// Get a list of third party identifiers associated with this account.
///
/// Returns contact information verified during registration.
#[endpoint]
async fn get(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<ThreepidsResBody> {
    let authed = depot.authed_info()?;
    let entries = crate::data::user::get_threepids(authed.user_id()).await?;
    json_ok(ThreepidsResBody::new(entries.into_iter().map(|entry| crate::core::third_party::ThirdPartyIdentifier {
        medium: entry.medium.into(), address: entry.address, added_at: entry.added_at, validated_at: entry.validated_at,
    }).collect()))
}

/// #POST /_matrix/client/v3/account/3pid/add
///
/// - 403 signals that the homeserver does not allow the third party identifier as a contact option.
#[endpoint]
async fn add(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Third party identifier is not allowed").into())
}

/// #POST /_matrix/client/v3/account/3pid/bind
///
/// - 403 signals that the homeserver does not allow the third party identifier as a contact option.
#[endpoint]
async fn bind(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Third party identifier is not allowed").into())
}

/// #POST /_matrix/client/v3/account/3pid/unbind
///
/// - `M_THREEPID_NOT_FOUND`: this server stores no third-party identifiers, so there is nothing to
///   unbind.
#[endpoint]
async fn unbind(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Contact changes are not supported. Contact your server administrator.").into())
}

/// #POST /_matrix/client/v3/account/3pid/delete
///
/// - `M_THREEPID_NOT_FOUND`: this server stores no third-party identifiers, so there is nothing to
///   delete.
#[endpoint]
async fn delete(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_denied("Contact changes are not supported. Contact your server administrator.").into())
}
