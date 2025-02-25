use salvo::prelude::*;

use crate::{AuthArgs, EmptyResult, empty_ok};

pub fn router() -> Router {
    Router::with_path("pubkey")
        .get(public_key)
        .push(Router::with_path("isvalid").get(is_valid))
        .push(Router::with_path("ephemeral/isvalid").get(ephemeral_is_valid))
}

#[endpoint]
async fn public_key(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}

#[endpoint]
async fn is_valid(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}

#[endpoint]
async fn ephemeral_is_valid(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}
