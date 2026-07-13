//! `POST /_matrix/client/*/account/3pid/add`
//!
//! Add contact information to a user's account
//!
//! `/v3/` ([spec])
//!
//! [spec]: https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3account3pidadd
//!
//! This homeserver does not support third-party identifiers, so the mutating
//! endpoints report that honestly instead of returning a fake success:
//! `add`/`bind` are denied (matching the `requestToken` endpoints) and
//! `delete`/`unbind` report that no such 3PID exists.

use salvo::prelude::*;

use crate::core::client::account::threepid::ThreepidsResBody;
use crate::{AuthArgs, EmptyResult, JsonResult, MatrixError, json_ok};

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
/// - Always empty: this server does not store third-party identifiers.
#[endpoint]
async fn get(_aa: AuthArgs) -> JsonResult<ThreepidsResBody> {
    json_ok(ThreepidsResBody::new(Vec::new()))
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
    Err(MatrixError::threepid_not_found("User has no third-party identifiers.").into())
}

/// #POST /_matrix/client/v3/account/3pid/delete
///
/// - `M_THREEPID_NOT_FOUND`: this server stores no third-party identifiers, so there is nothing to
///   delete.
#[endpoint]
async fn delete(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::threepid_not_found("User has no third-party identifiers.").into())
}
