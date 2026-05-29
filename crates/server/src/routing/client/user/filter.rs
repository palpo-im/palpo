use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::core::client::filter::{CreateFilterReqBody, CreateFilterResBody, FilterResBody};
use crate::{AuthArgs, DepotExt, JsonResult, data, json_ok};

/// #GET /_matrix/client/r0/user/{user_id}/filter/{filter_id}
/// Loads a filter that was previously created.
///
/// - A user can only access their own filters
#[endpoint]
pub(super) async fn get_filter(
    _aa: AuthArgs,
    filter_id: PathParam<i64>,
    depot: &mut Depot,
) -> JsonResult<FilterResBody> {
    let authed = depot.authed_info()?;
    let filter = crate::data::user::get_filter(authed.user_id(), filter_id.into_inner()).await?;
    json_ok(FilterResBody::new(filter))
}

/// #POST /_matrix/client/r0/user/{user_id}/filter
/// Creates a new filter to be used by other endpoints.
#[endpoint]
pub(super) async fn create_filter(
    _aa: AuthArgs,
    body: JsonBody<CreateFilterReqBody>,
    depot: &mut Depot,
) -> JsonResult<CreateFilterResBody> {
    let authed = depot.authed_info()?;
    let filter_id = data::user::create_filter(authed.user_id(), &body.filter).await?;
    json_ok(CreateFilterResBody::new(filter_id.to_string()))
}
