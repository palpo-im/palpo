use salvo::prelude::*;

use crate::core::client::rtc::transports::{RtcTransportsResBody, StableRtcTransportsResBody};
use crate::{AuthArgs, JsonResult, MatrixError, config, json_ok};

#[endpoint]
pub(super) fn stable_transports(_aa: AuthArgs) -> JsonResult<StableRtcTransportsResBody> {
    ensure_enabled()?;
    json_ok(StableRtcTransportsResBody {
        transports: config::get().rtc.transports.clone(),
    })
}

#[endpoint]
pub(super) fn unstable_transports(_aa: AuthArgs) -> JsonResult<RtcTransportsResBody> {
    ensure_enabled()?;
    json_ok(RtcTransportsResBody::new(
        config::get().rtc.transports.clone(),
    ))
}

fn ensure_enabled() -> Result<(), MatrixError> {
    if config::get().rtc.enabled {
        Ok(())
    } else {
        Err(MatrixError::not_found(
            "MatrixRTC transport discovery is disabled",
        ))
    }
}

#[cfg(test)]
#[path = "rtc_tests.rs"]
mod tests;
