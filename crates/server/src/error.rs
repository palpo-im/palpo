use std::borrow::Cow;
use std::io;
use std::string::FromUtf8Error;
use std::sync::OnceLock;

use async_trait::async_trait;
use salvo::http::{Method, StatusCode, StatusError};
use salvo::oapi::{self, EndpointOutRegister, ToSchema};
use salvo::prelude::{Depot, Request, Response, Writer};
use thiserror::Error;

// use crate::User;
// use crate::DepotExt;
use crate::core::MatrixError;
use crate::core::events::room::power_levels::PowerLevelsError;
use crate::core::state::StateError;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("public: `{0}`")]
    Public(String),
    #[error("internal: `{0}`")]
    Internal(String),
    #[error("state: `{0}`")]
    State(#[from] StateError),
    #[error("power levels: `{0}`")]
    PowerLevels(#[from] PowerLevelsError),
    // #[error("local unable process: `{0}`")]
    // LocalUnableProcess(String),
    #[error("salvo internal error: `{0}`")]
    Salvo(#[from] ::salvo::Error),
    #[error("parse int error: `{0}`")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("frequently request resource")]
    FrequentlyRequest,
    #[error("io: `{0}`")]
    Io(#[from] io::Error),
    #[error("utf8: `{0}`")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("decoding: `{0}`")]
    Decoding(Cow<'static, str>),
    #[error("url parse: `{0}`")]
    UrlParse(#[from] url::ParseError),
    #[error("serde json: `{0}`")]
    SerdeJson(#[from] serde_json::error::Error),
    #[error("diesel: `{0}`")]
    Diesel(#[from] diesel::result::Error),
    #[error("regex: `{0}`")]
    Regex(#[from] regex::Error),
    #[error("http: `{0}`")]
    HttpStatus(#[from] salvo::http::StatusError),
    #[error("http parse: `{0}`")]
    HttpParse(#[from] salvo::http::ParseError),
    #[error("reqwest: `{0}`")]
    Reqwest(#[from] reqwest::Error),
    #[error("data: `{0}`")]
    Data(#[from] crate::data::DataError),
    #[error("pool: `{0}`")]
    Pool(#[from] crate::data::PoolError),
    #[error("utf8: `{0}`")]
    Utf8Error(#[from] std::str::Utf8Error),
    // #[error("redis: `{0}`")]
    // Redis(#[from] redis::RedisError),
    #[error("GlobError error: `{0}`")]
    Glob(#[from] globwalk::GlobError),
    #[error("Matrix error: `{0}`")]
    Matrix(#[from] palpo_core::MatrixError),
    #[error("argon2 error: `{0}`")]
    Argon2(#[from] argon2::Error),
    #[error("Uiaa error: `{0}`")]
    Uiaa(#[from] palpo_core::client::uiaa::UiaaInfo),
    #[error("Send error: `{0}`")]
    Send(#[from] palpo_core::sending::SendError),
    #[error("ID parse error: `{0}`")]
    IdParse(#[from] palpo_core::identifiers::IdParseError),
    #[error("CanonicalJson error: `{0}`")]
    CanonicalJson(#[from] palpo_core::serde::CanonicalJsonError),
    #[error("MxcUriError: `{0}`")]
    MxcUri(#[from] palpo_core::identifiers::MxcUriError),
    #[error("ImageError: `{0}`")]
    Image(#[from] image::ImageError),
    #[error("Signatures: `{0}`")]
    Signatures(#[from] palpo_core::signatures::Error),
    #[error("FmtError: `{0}`")]
    Fmt(#[from] std::fmt::Error),
    #[error("CargoTomlError: `{0}`")]
    CargoToml(#[from] cargo_toml::Error),
    #[error("YamlError: `{0}`")]
    Yaml(#[from] serde_saphyr::ser_error::Error),
    #[error("Command error: `{0}`")]
    Clap(#[from] clap::Error),
    #[error("SystemTimeError: `{0}`")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("ReqwestMiddlewareError: `{0}`")]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("OpenDAL error: `{0}`")]
    OpenDal(#[from] opendal::Error),
}

impl AppError {
    pub fn public<S: Into<String>>(msg: S) -> Self {
        Self::Public(msg.into())
    }

    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }
    // pub fn local_unable_process<S: Into<String>>(msg: S) -> Self {
    //     Self::LocalUnableProcess(msg.into())
    // }

    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Diesel(diesel::result::Error::NotFound) => true,
            Self::Data(crate::data::DataError::Diesel(diesel::result::Error::NotFound)) => true,
            Self::Data(crate::data::DataError::Matrix(e)) => e.is_not_found(),
            Self::Matrix(e) => e.is_not_found(),
            _ => false,
        }
    }
}

/// Best-effort removal of `access_token` values from text destined for logs.
/// Reqwest errors embed the full request URL, which for appservice requests
/// carries the homeserver token as a query parameter.
fn redact_access_tokens(text: &str) -> Cow<'_, str> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?i)(access_token=)[^&\s'"]+"#).expect("static regex is valid")
    });
    re.replace_all(text, "${1}[redacted]")
}

/// The message returned to clients for errors without a specific Matrix
/// mapping. Debug builds return the underlying error text for diagnosability;
/// release builds return a stable category message so internal paths, object
/// store errors and upstream details stay in the server log only.
fn unhandled_client_message(e: &AppError, debug: bool) -> String {
    if debug {
        // Redact token-bearing URLs (reqwest Display embeds the request URL)
        // even in debug builds.
        return redact_access_tokens(&e.to_string()).into_owned();
    }
    match e {
        AppError::Reqwest(_) | AppError::ReqwestMiddleware(_) | AppError::Send(_) => {
            "failed to reach remote server".to_owned()
        }
        AppError::Pool(_) | AppError::OpenDal(_) => "internal storage error".to_owned(),
        _ => "internal server error".to_owned(),
    }
}

fn expose_diesel_not_found(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::DELETE)
}

fn internal_db_error() -> MatrixError {
    let mut error = MatrixError::unknown("unknown db error");
    error.status_code = Some(StatusCode::INTERNAL_SERVER_ERROR);
    error
}

fn matrix_error_from_diesel(method: &Method, error: &diesel::result::Error) -> MatrixError {
    match error {
        diesel::result::Error::NotFound if expose_diesel_not_found(method) => {
            tracing::warn!(%method, "diesel not found");
            MatrixError::not_found("resource not found")
        }
        diesel::result::Error::NotFound => {
            tracing::error!(%method, "unexpected diesel not found");
            internal_db_error()
        }
        _ => {
            tracing::error!(%method, error = ?error, "diesel db error");
            internal_db_error()
        }
    }
}

#[async_trait]
impl Writer for AppError {
    async fn write(mut self, req: &mut Request, depot: &mut Depot, res: &mut Response) {
        let matrix = match self {
            Self::Salvo(_e) => MatrixError::unknown("unknown error in salvo"),
            Self::FrequentlyRequest => MatrixError::unknown("frequently request resource"),
            Self::Public(msg) => MatrixError::unknown(msg),
            Self::Internal(msg) => {
                error!(error = %msg, "internal error");
                if cfg!(debug_assertions) {
                    MatrixError::unknown(format!("internal error: {msg}"))
                } else {
                    // Internal messages may carry paths, queries or upstream
                    // details; production clients get a stable message.
                    MatrixError::unknown("internal server error")
                }
            }
            // Self::LocalUnableProcess(msg) => MatrixError::unrecognized(msg),
            Self::Matrix(e) => e,
            Self::State(e) => {
                if let StateError::Forbidden(msg) = e {
                    tracing::error!(error = ?msg, "forbidden error");
                    MatrixError::forbidden(msg, None)
                } else if let StateError::AuthEvent(msg) = e {
                    tracing::error!(error = ?msg, "forbidden error");
                    MatrixError::forbidden(msg, None)
                } else {
                    MatrixError::unknown(e.to_string())
                }
            }
            Self::Uiaa(uiaa) => {
                use crate::core::client::uiaa::ErrorKind;
                if res.status_code.map(|c| c.is_success()).unwrap_or(true) {
                    let code = if let Some(error) = &uiaa.auth_error {
                        match &error.kind {
                            ErrorKind::Forbidden | ErrorKind::UserDeactivated => {
                                StatusCode::FORBIDDEN
                            }
                            ErrorKind::NotFound => StatusCode::NOT_FOUND,
                            ErrorKind::BadStatus { status, .. } => {
                                status.unwrap_or(StatusCode::BAD_REQUEST)
                            }
                            ErrorKind::BadState | ErrorKind::BadJson | ErrorKind::BadAlias => {
                                StatusCode::BAD_REQUEST
                            }
                            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
                            ErrorKind::CannotOverwriteMedia => StatusCode::CONFLICT,
                            ErrorKind::NotYetUploaded => StatusCode::GATEWAY_TIMEOUT,
                            _ => StatusCode::INTERNAL_SERVER_ERROR,
                        }
                    } else {
                        StatusCode::UNAUTHORIZED
                    };
                    res.status_code(code);
                }
                res.add_header(salvo::http::header::CONTENT_TYPE, "application/json", true)
                    .ok();
                let body: Vec<u8> = crate::core::serde::json_to_buf(&uiaa).unwrap();
                res.write_body(body).ok();
                return;
            }
            Self::Diesel(e) => matrix_error_from_diesel(req.method(), &e),
            Self::HttpStatus(e) => match e.code {
                StatusCode::NOT_FOUND => MatrixError::not_found(e.brief),
                StatusCode::FORBIDDEN => MatrixError::forbidden(e.brief, None),
                StatusCode::UNAUTHORIZED => MatrixError::unauthorized(e.brief),
                code => {
                    let mut e = MatrixError::unknown(e.brief);
                    e.status_code = Some(code);
                    e
                }
            },
            Self::Data(e) => {
                e.write(req, depot, res).await;
                return;
            }
            e => {
                // These are unexpected errors that aren't mapped to a specific Matrix
                // error. Log the full detail at error level (with token-bearing URLs
                // redacted); what reaches the client is decided by
                // `unhandled_client_message` — full text in debug builds, a stable
                // category message in release builds.
                tracing::error!(
                    error = %redact_access_tokens(&format!("{e:?}")),
                    error_display = %redact_access_tokens(&e.to_string()),
                    "unhandled application error"
                );
                let is_upstream = matches!(
                    e,
                    Self::Reqwest(_)
                        | Self::ReqwestMiddleware(_)
                        | Self::Pool(_)
                        | Self::Send(_)
                        | Self::OpenDal(_)
                );
                let message = unhandled_client_message(&e, cfg!(debug_assertions));
                let mut matrix = MatrixError::unknown(message);
                if is_upstream {
                    // Failures talking to an upstream (federation peer, object store,
                    // db pool) are gateway errors, not malformed client requests.
                    matrix.status_code = Some(StatusCode::BAD_GATEWAY);
                }
                matrix
            }
        };
        matrix.write(req, depot, res).await;
    }
}
impl EndpointOutRegister for AppError {
    fn register(components: &mut oapi::Components, operation: &mut oapi::Operation) {
        operation.responses.insert(
            StatusCode::INTERNAL_SERVER_ERROR.as_str(),
            oapi::Response::new("Internal server error")
                .add_content("application/json", StatusError::to_schema(components)),
        );
        operation.responses.insert(
            StatusCode::NOT_FOUND.as_str(),
            oapi::Response::new("Not found")
                .add_content("application/json", StatusError::to_schema(components)),
        );
        operation.responses.insert(
            StatusCode::BAD_REQUEST.as_str(),
            oapi::Response::new("Bad request")
                .add_content("application/json", StatusError::to_schema(components)),
        );
    }
}

#[cfg(test)]
mod tests {
    use diesel::result::{DatabaseErrorKind, Error as DieselError};
    use salvo::http::Method;

    use super::*;

    #[test]
    fn access_tokens_are_redacted_from_log_text() {
        assert_eq!(
            redact_access_tokens(
                "error sending request for url \
                 (https://as.example/txn/1?access_token=secret123&ts=5): timed out"
            ),
            "error sending request for url \
             (https://as.example/txn/1?access_token=[redacted]&ts=5): timed out"
        );
        assert_eq!(
            redact_access_tokens("Access_Token=AbC.123-x\" and more"),
            "Access_Token=[redacted]\" and more"
        );
        assert_eq!(redact_access_tokens("no tokens here"), "no tokens here");
    }

    #[test]
    fn release_client_message_is_generic() {
        let io_error = AppError::Io(io::Error::other("/var/lib/palpo/secret-path: boom"));
        assert_eq!(
            unhandled_client_message(&io_error, false),
            "internal server error"
        );
        // Debug builds keep the underlying detail for diagnosability.
        assert!(unhandled_client_message(&io_error, true).contains("secret-path"));
    }

    #[tokio::test]
    async fn get_not_found_stays_404() {
        let mut req = Request::new();
        *req.method_mut() = Method::GET;
        let mut depot = Depot::new();
        let mut res = Response::new();

        AppError::Diesel(DieselError::NotFound)
            .write(&mut req, &mut depot, &mut res)
            .await;

        assert_eq!(res.status_code, Some(StatusCode::NOT_FOUND));
    }

    #[test]
    fn nested_data_not_found_is_recognized() {
        let error = AppError::Data(crate::data::DataError::Diesel(DieselError::NotFound));

        assert!(error.is_not_found());
    }

    #[tokio::test]
    async fn post_not_found_is_not_404() {
        let mut req = Request::new();
        *req.method_mut() = Method::POST;
        let mut depot = Depot::new();
        let mut res = Response::new();

        AppError::Diesel(DieselError::NotFound)
            .write(&mut req, &mut depot, &mut res)
            .await;

        assert_eq!(res.status_code, Some(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[tokio::test]
    async fn database_errors_return_500() {
        let mut req = Request::new();
        *req.method_mut() = Method::GET;
        let mut depot = Depot::new();
        let mut res = Response::new();

        AppError::Diesel(DieselError::DatabaseError(
            DatabaseErrorKind::Unknown,
            Box::new(String::from("boom")),
        ))
        .write(&mut req, &mut depot, &mut res)
        .await;

        assert_eq!(res.status_code, Some(StatusCode::INTERNAL_SERVER_ERROR));
    }
}
