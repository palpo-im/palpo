use std::time::{Duration, SystemTime};

use base64::Engine;
use hmac::{Hmac, Mac};
use salvo::prelude::*;
use sha1::Sha1;

use crate::core::UnixSeconds;
use crate::core::client::voip::TurnServerResBody;
use crate::{AuthArgs, DepotExt, JsonResult, MatrixError, config, hoops, json_ok};

type HmacSha1 = Hmac<Sha1>;

pub fn authed_router() -> Router {
    Router::with_path("voip/turnServer")
        .hoop(hoops::limit_rate)
        .get(turn_server)
}

/// #GET /_matrix/client/r0/voip/turnServer
/// TODO: Returns information about the recommended turn server.
#[endpoint]
async fn turn_server(_aa: AuthArgs, depot: &mut Depot) -> JsonResult<TurnServerResBody> {
    let authed = depot.authed_info()?;

    let conf = config::get();
    let turn_conf = conf
        .enabled_turn()
        .ok_or_else(|| MatrixError::not_found("TURN server is not configured"))?;

    // MSC4166: return M_NOT_FOUND 404 if no TURN URIs are specified in any way
    if turn_conf.uris.is_empty() {
        return Err(MatrixError::not_found("turn_uris is empty").into());
    }

    let turn_secret = turn_conf.secret.clone();

    let (username, password) = if !turn_secret.is_empty() {
        let expiry =
            UnixSeconds::from_system_time(SystemTime::now() + Duration::from_secs(turn_conf.ttl))
                .expect("time is valid");

        let username = format!("{}:{}", expiry.get(), authed.user_id());

        let mut mac = HmacSha1::new_from_slice(turn_secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(username.as_bytes());

        let password: String =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        (username, password)
    } else {
        (turn_conf.username.clone(), turn_conf.password.clone())
    };

    json_ok(TurnServerResBody {
        username,
        password,
        uris: turn_conf.uris.clone(),
        ttl: Duration::from_secs(turn_conf.ttl),
    })
}
