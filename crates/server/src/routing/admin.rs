mod event;
mod event_report;
mod federation;
mod mas;
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

/// Middleware to authenticate MAS requests via shared secret
#[handler]
pub async fn auth_by_mas_secret(aa: crate::AuthArgs) -> AppResult<()> {
    let token = aa
        .require_access_token()
        .map_err(crate::AppError::from)?;
    let conf = crate::config::get();
    let Some(mas_secret) = &conf.admin.mas_secret else {
        return Err(
            MatrixError::forbidden("MAS endpoints are not configured on this server", None).into(),
        );
    };
    if token != mas_secret.as_str() {
        return Err(MatrixError::forbidden("Invalid MAS secret", None).into());
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
    // MAS modern endpoints - separate auth via shared secret
    admin = admin.push(
        Router::with_path("_synapse/mas")
            .hoop(auth_by_mas_secret)
            .push(mas::router()),
    );
    admin
}

#[handler]
async fn home() -> &'static str {
    "Palpo Admin API"
}
