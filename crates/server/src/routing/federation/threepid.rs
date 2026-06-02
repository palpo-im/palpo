use salvo::prelude::*;

use crate::{AuthArgs, EmptyResult, MatrixError};

pub fn router() -> Router {
    Router::with_path("3pid/onbind").put(on_bind)
}

#[endpoint]
async fn on_bind(_aa: AuthArgs) -> EmptyResult {
    Err(MatrixError::unrecognized("3PID bind notifications are not implemented.").into())
}
