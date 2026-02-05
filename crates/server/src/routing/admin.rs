mod event;
mod event_report;
mod federation;
mod media;
mod register;
mod registration_token;
mod room;
mod scheduled_task;
mod server_notice;
mod statistic;
mod user;
mod user_admin;
mod user_lookup;

use salvo::prelude::*;

use crate::routing::prelude::*;

/// Middleware to require admin privileges
#[handler]
pub async fn require_admin(depot: &mut Depot) -> AppResult<()> {
    let authed = depot.authed_info()?;
    if !authed.is_admin() {
        return Err(MatrixError::forbidden("Requires admin privileges", None).into());
    }
    Ok(())
}

pub fn router() -> Router {
    let mut admin = Router::new().oapi_tag("admin");
    for v in ["_palpo/admin", "_synapse/admin"] {
        admin = admin.push(
            Router::with_path(v)
                .hoop(crate::hoops::auth_by_access_token)
                .hoop(require_admin)
                .get(home)
                .push(event::router())
                .push(event_report::router())
                .push(federation::router())
                .push(media::router())
                .push(register::router())
                .push(registration_token::router())
                .push(room::router())
                .push(scheduled_task::router())
                .push(server_notice::router())
                .push(statistic::router())
                .push(user::router())
                .push(user_admin::router())
                .push(user_lookup::router()),
        )
    }
    admin
}

#[handler]
async fn home() -> &'static str {
    "Palpo Admin API"
}
