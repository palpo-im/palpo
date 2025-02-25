use salvo::prelude::*;

use crate::{AuthArgs, EmptyResult, empty_ok};

pub fn router() -> Router {
    Router::with_path("3pid")
        .push(Router::with_path("bind").post(bind))
        .push(Router::with_path("unbind").post(unbind))
        .push(Router::with_path("getValidated3pid").get(validated))
}

#[endpoint]
async fn bind(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}
#[endpoint]
async fn unbind(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}

#[endpoint]
async fn validated(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}
