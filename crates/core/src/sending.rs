use std::future::Future;
use std::ops::{Deref, DerefMut};

use reqwest::{Client as ReqwestClient, ClientBuilder, Request as ReqwestRequest};
use salvo::http::header::CONTENT_TYPE;
use salvo::http::{HeaderName, HeaderValue, Method};
use serde::Deserialize;
use thiserror::Error;
use url::{ParseError, Url};

pub fn client() -> ReqwestClient {
    ClientBuilder::new().build().unwrap()
}

#[derive(Debug)]
pub struct SendRequest {
    inner: ReqwestRequest,
}

#[macro_export]
macro_rules! json_body_modifier {
    ($name:ident) => {
        impl $crate::sending::SendModifier for $name {
            fn modify(
                self,
                request: &mut $crate::sending::SendRequest,
            ) -> Result<(), $crate::sending::SendError> {
                let bytes = serde_json::to_vec(&self)?;
                *request.body_mut() = Some(bytes.into());
                Ok(())
            }
        }
    };
}
macro_rules! method {
    ($name:ident, $method:ident) => {
        pub fn $name(url: Url) -> SendRequest {
            SendRequest {
                inner: ReqwestRequest::new(Method::$method, url),
            }
        }
    };
}
method!(get, GET);
method!(patch, PATCH);
method!(put, PUT);
method!(post, POST);
method!(delete, DELETE);

#[derive(Error, Debug)]
pub enum SendError {
    #[error("parse url: `{0}`")]
    Url(#[from] ParseError),
    #[error("reqwest: `{0}`")]
    Reqwest(#[from] reqwest::Error),
    #[error("json: `{0}`")]
    Json(#[from] serde_json::Error),
    #[error("other: `{0}`")]
    Other(String),
}

impl SendError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

pub type SendResult<T> = Result<T, SendError>;

impl SendRequest {
    method!(get, GET);
    method!(patch, PATCH);
    method!(put, PUT);
    method!(post, POST);
    method!(delete, DELETE);

    pub fn into_inner(self) -> reqwest::Request {
        self.inner
    }
    pub fn stuff(mut self, modifier: impl SendModifier) -> Result<Self, SendError> {
        modifier.modify(&mut self)?;
        if !self.headers().contains_key(CONTENT_TYPE) {
            self.headers_mut()
                .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        }
        Ok(self)
    }

    pub async fn load<R>(self) -> Result<R, SendError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let res = client().execute(self.inner).await?;
        res.json().await.map_err(SendError::Reqwest)
    }

    pub async fn load_by_client<R>(self, client: ReqwestClient) -> Result<R, SendError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let res = client.execute(self.inner).await?;
        res.json().await.map_err(SendError::Reqwest)
    }

    pub async fn send<R>(self) -> Result<R, SendError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let res = client().execute(self.inner).await?;
        res.json().await.map_err(SendError::Reqwest)
    }

    pub async fn send_by_client<R>(self, client: ReqwestClient) -> Result<R, SendError>
    where
        R: for<'de> Deserialize<'de>,
    {
        let res = client.execute(self.inner).await?;
        res.json().await.map_err(SendError::Reqwest)
    }

    pub fn exec(self) -> impl Future<Output = Result<reqwest::Response, reqwest::Error>> {
        client().execute(self.inner)
    }
}

impl Deref for SendRequest {
    type Target = ReqwestRequest;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for SendRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

pub trait SendModifier {
    fn modify(self, request: &mut SendRequest) -> Result<(), SendError>;
}

impl SendModifier for (HeaderName, HeaderValue) {
    fn modify(self, request: &mut SendRequest) -> Result<(), SendError> {
        request.headers_mut().append(self.0, self.1);
        Ok(())
    }
}
