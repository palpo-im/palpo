use salvo::prelude::*;

use crate::{AuthArgs, EmptyResult, empty_ok};

pub fn router() -> Router {
    Router::with_path("transactions/{txn_id}").put(send_event)
}

#[endpoint]
async fn send_event(_aa: AuthArgs) -> EmptyResult {
    // TODO: todo
    empty_ok()
}
