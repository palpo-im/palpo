use salvo::oapi::extract::*;
use salvo::prelude::*;
use serde::Serialize;

use crate::{AuthArgs, JsonResult, MatrixError, json_ok, user};

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
