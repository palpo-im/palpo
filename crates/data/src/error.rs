use std::borrow::Cow;
use std::io;
use std::string::FromUtf8Error;

use async_trait::async_trait;
use palpo_core::MatrixError;
use salvo::http::{StatusCode, StatusError};
use salvo::oapi::{self, EndpointOutRegister, ToSchema};
use salvo::prelude::{Depot, Request, Response, Writer};
use thiserror::Error;
// use crate::User;
// use crate::DepotExt;

#[derive(Error, Debug)]
pub enum DataError {
    #[error("public: `{0}`")]
    Public(String),
    #[error("internal: `{0}`")]
    Internal(String),
    #[error("parse int error: `{0}`")]
    ParseIntError(#[from] std::num::ParseIntError),
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
    #[error("pool: `{0}`")]
    Pool(#[from] crate::PoolError),
    #[error("utf8: `{0}`")]
    Utf8Error(#[from] std::str::Utf8Error),
    #[error("Matrix error: `{0}`")]
    Matrix(#[from] palpo_core::MatrixError),
    #[error("Uiaa error: `{0}`")]
    Uiaa(#[from] palpo_core::client::uiaa::UiaaInfo),
    #[error("Send error: `{0}`")]
    Send(#[from] palpo_core::sending::SendError),
    #[error("ID parse error: `{0}`")]
    IdParse(#[from] palpo_core::identifiers::IdParseError),
    #[error("CanonicalJson error: `{0}`")]
    CanonicalJson(#[from] palpo_core::serde::CanonicalJsonError),
    #[error("MxcUriError: `{0}`")]
    MxcUriError(#[from] palpo_core::identifiers::MxcUriError),
    #[error("ImageError: `{0}`")]
    ImageError(#[from] image::ImageError),
    #[error("Signatures: `{0}`")]
    Signatures(#[from] palpo_core::signatures::Error),
}

impl DataError {
    pub fn public<S: Into<String>>(msg: S) -> Self {
        Self::Public(msg.into())
    }

    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }
}

#[async_trait]
impl Writer for DataError {
    async fn write(mut self, req: &mut Request, depot: &mut Depot, res: &mut Response) {
        let matrix = match self {
            Self::Public(msg) => MatrixError::unknown(msg),
            Self::Internal(_msg) => MatrixError::unknown("Unknown data internal error."),
            Self::Matrix(e) => e,
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
            Self::Diesel(e) => {
                tracing::error!(error = ?e, "diesel db error");
                if let diesel::result::Error::NotFound = e {
                    MatrixError::not_found("data resource not found")
                } else {
                    MatrixError::unknown("unknown db error")
                }
            }
            _ => MatrixError::unknown("unknown data error happened"),
        };
        matrix.write(req, depot, res).await;
    }
}
impl EndpointOutRegister for DataError {
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
