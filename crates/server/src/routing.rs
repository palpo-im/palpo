mod admin;
mod appservice;
mod client;
mod federation;
mod media;

use salvo::prelude::*;
use salvo::serve_static::StaticDir;

use crate::core::MatrixError;
use crate::core::client::discovery::client::{ClientResBody, HomeServerInfo};
use crate::core::client::discovery::support::{Contact, SupportResBody};
use crate::core::federation::directory::ServerResBody;
use crate::{AppResult, JsonResult, config, hoops, json_ok};

pub mod prelude {
    pub use salvo::prelude::*;

    pub use crate::core::MatrixError;
    pub use crate::core::identifiers::*;
    pub use crate::core::serde::{JsonValue, RawJson};
    pub use crate::exts::*;
    pub use crate::{
        AppError, AppResult, AuthArgs, DepotExt, EmptyResult, JsonResult, OptionalExtension,
        config, empty_ok, hoops, json_ok,
    };
}

pub fn root() -> Router {
    Router::new()
        .hoop(hoops::ensure_accept)
        .hoop(hoops::ensure_content_type)
        .hoop(hoops::limit_size)
        .get(home)
        .push(
            Router::with_path("_matrix")
                .push(client::router())
                .push(media::router())
                .push(federation::router())
                .push(federation::key::router())
                .push(appservice::router()),
        )
        .push(admin::router())
        .push(
            Router::with_path(".well-known/matrix")
                .push(Router::with_path("client").get(well_known_client))
                .push(Router::with_path("support").get(well_known_support))
                .push(Router::with_path("server").get(well_known_server)),
        )
        .push(Router::with_path("{*path}").get(StaticDir::new("./static")))
}

#[handler]
async fn home(req: &mut Request, res: &mut Response) {
    if let Some(home_page) = &config::get().home_page {
        res.send_file(home_page, req.headers()).await;
    }
    res.render("Hello Palpo");
}

#[handler]
pub async fn limit_rate() -> AppResult<()> {
    Ok(())
}

#[endpoint]
fn well_known_client() -> JsonResult<ClientResBody> {
    let conf = config::get();
    let client_url = conf.well_known_client();
    json_ok(ClientResBody::new(HomeServerInfo {
        base_url: client_url.clone(),
    }))
}

#[endpoint]
fn well_known_support() -> JsonResult<SupportResBody> {
    let conf = config::get();
    let support_page = conf
        .well_known
        .support_page
        .as_ref()
        .map(ToString::to_string);

    let role = conf.well_known.support_role.clone();

    // support page or role must be either defined for this to be valid
    if support_page.is_none() && role.is_none() {
        return Err(MatrixError::not_found("Not found.").into());
    }

    let email_address = conf.well_known.support_email.clone();

    let matrix_id = conf.well_known.support_mxid.clone();

    // if a role is specified, an email address or matrix id is required
    if role.is_some() && (email_address.is_none() && matrix_id.is_none()) {
        return Err(MatrixError::not_found("Not found.").into());
    }

    // TODO: support defining multiple contacts in the config
    let mut contacts: Vec<Contact> = vec![];

    if let Some(role) = role {
        let contact = Contact {
            role,
            email_address,
            matrix_id,
        };

        contacts.push(contact);
    }

    // support page or role+contacts must be either defined for this to be valid
    if contacts.is_empty() && support_page.is_none() {
        return Err(MatrixError::not_found("Not found.").into());
    }

    json_ok(SupportResBody {
        contacts,
        support_page,
    })
}

#[endpoint]
fn well_known_server() -> JsonResult<ServerResBody> {
    json_ok(ServerResBody {
        server: config::get().well_known_server(),
    })
}
