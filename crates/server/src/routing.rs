mod admin;
mod appservice;
mod client;
mod federation;
mod media;

use bytes::Bytes;
use salvo::http::StatusCode;
use salvo::http::header::{CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use salvo::serve_static::StaticDir;

use crate::core::MatrixError;
use crate::core::client::discovery::client::{AuthenticationInfo, ClientResBody, HomeServerInfo};
use crate::core::client::discovery::support::{Contact, SupportResBody};
use crate::core::federation::directory::ServerResBody;
use crate::{AppResult, JsonResult, config, hoops, json_ok, sending};

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
        match HomePageSource::from_config(home_page) {
            HomePageSource::Remote(url) => {
                if let Some((body, content_type)) = fetch_remote_home_page(url).await {
                    res.status_code(StatusCode::OK);
                    res.headers_mut().insert(
                        CONTENT_TYPE,
                        HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
                            HeaderValue::from_static("text/html; charset=utf-8")
                        }),
                    );
                    let _ = res.write_body(body);
                    return;
                }
            }
            HomePageSource::Local(path) => {
                res.send_file(path, req.headers()).await;
                return;
            }
        }
    }

    res.status_code(StatusCode::OK);
    res.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    let _ = res.write_body("Hello Palpo");
}

enum HomePageSource<'a> {
    Local(&'a str),
    Remote(&'a str),
}

impl<'a> HomePageSource<'a> {
    fn from_config(value: &'a str) -> Self {
        if value.starts_with("https://") {
            Self::Remote(value)
        } else {
            Self::Local(value)
        }
    }
}

async fn fetch_remote_home_page(url: &str) -> Option<(Bytes, String)> {
    let response = match sending::default_client()
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            crate::info::version::user_agent(),
        )
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(url, error = %error, "failed to fetch remote home page");
            return None;
        }
    };

    if !response.status().is_success() {
        tracing::warn!(
            url,
            status = %response.status(),
            "remote home page returned non-success status"
        );
        return None;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "text/html; charset=utf-8".to_owned());

    match response.bytes().await {
        Ok(body) => Some((body, content_type)),
        Err(error) => {
            tracing::warn!(url, error = %error, "failed to read remote home page body");
            None
        }
    }
}

#[handler]
pub async fn limit_rate() -> AppResult<()> {
    Ok(())
}

#[endpoint]
fn well_known_client() -> JsonResult<ClientResBody> {
    let conf = config::get();
    let client_url = conf.well_known_client();
    let mut body = ClientResBody::new(HomeServerInfo {
        base_url: client_url.clone(),
    });

    // Advertise OIDC issuer (MSC3861) — prefer delegated_auth, fall back to oidc.mas_issuer
    if let Some(da) = conf.enabled_delegated_auth() {
        if let Some(issuer) = &da.issuer {
            body.authentication = Some(AuthenticationInfo {
                issuer: issuer.clone(),
            });
        }
    } else if let Some(oidc) = conf.oidc.as_ref()
        && let Some(issuer) = &oidc.mas_issuer
    {
        body.authentication = Some(AuthenticationInfo {
            issuer: issuer.clone(),
        });
    }

    json_ok(body)
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

#[cfg(test)]
mod tests {
    use super::HomePageSource;

    #[test]
    fn home_page_classifies_https_urls_as_remote() {
        assert!(matches!(
            HomePageSource::from_config("https://example.com/index.html"),
            HomePageSource::Remote(_)
        ));
    }

    #[test]
    fn home_page_classifies_other_values_as_local_paths() {
        assert!(matches!(
            HomePageSource::from_config("./static/index.html"),
            HomePageSource::Local(_)
        ));
        assert!(matches!(
            HomePageSource::from_config("/data/workspace/index.html"),
            HomePageSource::Local(_)
        ));
    }
}

#[endpoint]
fn well_known_server() -> JsonResult<ServerResBody> {
    json_ok(ServerResBody {
        server: config::get().well_known_server(),
    })
}
