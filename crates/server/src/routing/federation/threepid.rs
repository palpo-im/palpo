use salvo::prelude::*;

use crate::{EmptyResult, MatrixError};

pub fn router() -> Router {
    // matrix-spec#2452 corrected the historical PUT typo to POST. Retain
    // PUT as a compatibility alias; neither method implements binding.
    Router::with_path("3pid/onbind").post(on_bind).put(on_bind)
}

#[endpoint]
async fn on_bind() -> EmptyResult {
    Err(MatrixError::unrecognized("3PID bind notifications are not implemented.").into())
}
