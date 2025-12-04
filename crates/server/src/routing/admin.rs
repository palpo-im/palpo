mod event;
mod federation;
mod media;
mod register;
mod room;
mod scheduled_task;
mod server_notice;
mod statistic;
mod user;
mod user_lookup;

use std::collections::BTreeMap;

use salvo::oapi::extract::*;
use salvo::prelude::*;

use crate::config;
use crate::core::client::discovery::{
    capabilities::{
        Capabilities, CapabilitiesResBody, ChangePasswordCapability, RoomVersionStability,
        RoomVersionsCapability, SetAvatarUrlCapability, SetDisplayNameCapability,
        ThirdPartyIdChangesCapability,
    },
    versions::VersionsResBody,
};
use crate::core::client::search::{ResultCategories, SearchReqArgs, SearchReqBody, SearchResBody};
use crate::routing::prelude::*;

pub fn router() -> Router {
    let mut admin = Router::new().oapi_tag("admin");
    for v in ["_palpo/admin", "_synapse/admin"] {
        admin = admin.push(
            Router::with_path(v)
                .get(home)
                .push(event::router())
                .push(federation::router())
                .push(media::router())
                .push(register::router())
                .push(room::router())
                .push(scheduled_task::router())
                .push(server_notice::router())
                .push(statistic::router())
                .push(user::router())
                .push(user_lookup::router()),
        )
    }
    admin
}

#[handler]
async fn home() -> &'static str {
    "Palpo Admin API"
}
