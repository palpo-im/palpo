use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::core::UnixMillis;
use crate::core::federation::authorization::{EventAuthReqArgs, EventAuthResBody};
use crate::core::federation::event::{
    EventReqArgs, EventResBody, MissingEventsReqBody, MissingEventsResBody,
};
use crate::core::identifiers::*;
use crate::core::room::{TimestampToEventReqArgs, TimestampToEventResBody};
use crate::data::room::DbEvent;
use crate::room::{state, timeline};
use crate::{
    AppError, AuthArgs, DepotExt, EmptyResult, JsonResult, MatrixError, config, empty_ok, json_ok,
    user,
};

pub fn router() -> Router {
    Router::new().push(Router::with_path("username_available").get(check_username_available))
}

#[derive(Serialize, ToSchema, Debug, Clone)]
struct AvailableResBody {
    available: bool,
}
/// An admin API to check if a given username is available, regardless of whether registration is
/// enabled.
#[endpoint]
fn check_username_available(
    _aa: AuthArgs,
    username: QueryParam<String, true>,
) -> JsonResult<AvailableResBody> {
    if !user::is_username_available(&username)? {
        Err(MatrixError::user_in_use("desired user id is invalid or already taken").into())
    } else {
        json_ok(AvailableResBody { available: true })
    }
}
