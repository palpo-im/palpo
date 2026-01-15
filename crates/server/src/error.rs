use std::borrow::Cow;
use std::io;
use std::string::FromUtf8Error;

use async_trait::async_trait;
use salvo::http::{StatusCode, StatusError};
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
            Self::Matrix(e) => e.is_not_found(),
            _ => false,
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
                error!(error = ?msg, "internal error");
                MatrixError::unknown("unknown error")
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
                            ErrorKind::Forbidden { .. } | ErrorKind::UserDeactivated => {
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
            Self::Diesel(e) => {
                tracing::error!(error = ?e, "diesel db error");
                if let diesel::result::Error::NotFound = e {
                    MatrixError::not_found("resource not found")
                } else {
                    MatrixError::unknown("unknown db error")
                }
            }
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
                tracing::error!(error = ?e, "unknown error");
                MatrixError::unknown("unknown error happened")
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
