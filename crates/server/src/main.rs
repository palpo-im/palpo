#![allow(dead_code, missing_docs)]
// #![deny(unused_crate_dependencies)]
// #[macro_use]
// extern crate diesel;
// extern crate dotenvy;
// #[macro_use]
// extern crate thiserror;
// #[macro_use]
// extern crate anyhow;
// #[macro_use]
// mod macros;
// #[macro_use]
// pub mod permission;

#[macro_use]
extern crate tracing;

pub mod auth;
pub mod config;
pub mod env_vars;
pub mod hoops;
pub mod routing;
pub mod utils;
pub use auth::{AuthArgs, AuthedInfo};
pub mod admin;
pub mod appservice;
pub mod directory;
pub mod event;
pub mod exts;
pub mod federation;
pub mod media;
pub mod membership;
pub mod room;
pub mod sending;
pub mod server_key;
pub mod state;
pub mod transaction_id;
pub mod uiaa;
pub mod user;
pub use exts::*;
mod cjson;
pub use cjson::Cjson;
mod signing_keys;
pub mod sync_v3;
pub mod sync_v5;
pub mod watcher;
pub use event::{PduBuilder, PduEvent, SnPduEvent};
pub use signing_keys::SigningKeys;
mod global;
pub use global::*;
mod info;
pub mod logging;

pub mod error;
pub use core::error::MatrixError;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
pub use diesel::result::Error as DieselError;
use dotenvy::dotenv;
pub use error::AppError;
use figment::providers::Env;
use salvo::catcher::Catcher;
use salvo::compression::{Compression, CompressionLevel};
use salvo::conn::rustls::{Keycert, RustlsConfig};
use salvo::conn::tcp::DynTcpAcceptors;
use salvo::cors::{self, AllowHeaders, Cors};
use salvo::http::Method;
use salvo::logging::Logger;
use salvo::prelude::*;
use tracing_futures::Instrument;
pub use {
    jsonwebtoken as jwt, palpo_core as core, palpo_data as data, palpo_server_macros as macros,
};

use crate::config::ServerConfig;

pub type AppResult<T> = Result<T, crate::AppError>;
pub type DieselResult<T> = Result<T, diesel::result::Error>;
pub type JsonResult<T> = Result<Json<T>, crate::AppError>;
pub type CjsonResult<T> = Result<Cjson<T>, crate::AppError>;
pub type EmptyResult = Result<Json<EmptyObject>, crate::AppError>;

pub fn json_ok<T>(data: T) -> JsonResult<T> {
    Ok(Json(data))
}
pub fn cjson_ok<T>(data: T) -> CjsonResult<T> {
    Ok(Cjson(data))
}
pub fn empty_ok() -> JsonResult<EmptyObject> {
    Ok(Json(EmptyObject {}))
}

pub trait OptionalExtension<T> {
    fn optional(self) -> AppResult<Option<T>>;
}

impl<T> OptionalExtension<T> for AppResult<T> {
    fn optional(self) -> AppResult<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(AppError::Matrix(e)) => {
                if e.is_not_found() {
                    Ok(None)
                } else {
                    Err(AppError::Matrix(e))
                }
            }
            Err(AppError::Diesel(diesel::result::Error::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Commandline arguments
#[derive(Parser, Debug)]
#[clap(
	about,
	long_about = None,
	name = "palpo",
	version = crate::info::version(),
)]
pub(crate) struct Args {
    #[arg(short, long)]
    /// Path to the config TOML file (optional)
    pub(crate) config: Option<PathBuf>,

    /// Activate admin command console automatically after startup.
    #[arg(long, num_args(0))]
    pub(crate) console: bool,

    #[arg(long, short, num_args(1), default_value_t = true)]
    pub(crate) server: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if dotenvy::from_filename(".env.local").is_err() {
        tracing::debug!(".env.local file is not found");
    }
    if let Err(e) = dotenv() {
        tracing::info!("dotenv not loaded: {:?}", e);
    }

    let args = Args::parse();
    println!("Args: {:?}", args);

    let config_path = if let Some(config) = &args.config {
        config
    } else {
        &PathBuf::from(
            Env::var("PALPO_CONFIG").unwrap_or_else(|| utils::select_config_path().into()),
        )
    };

    crate::config::init(config_path);
    let conf = crate::config::get();
    conf.check().expect("config is not valid!");

    crate::logging::init()?;
    crate::data::init(&conf.db.clone().into_data_db_config());

    if args.console {
        tracing::info!("starting admin console...");

        if !args.server {
            crate::admin::start()
                .await
                .expect("admin console failed to start");
            tracing::info!("admin console stopped");
            return Ok(());
        } else {
            tokio::spawn(async move {
                crate::admin::start()
                    .await
                    .expect("admin console failed to start");
                tracing::info!("admin console stopped");
            });
        }
    }
    if !args.server {
        tracing::info!("server is not started, exiting...");
        return Ok(());
    }

    crate::sending::guard::start();

    let router = routing::root();
    // let doc = OpenApi::new("palpo api", "0.0.1").merge_router(&router);
    // let router = router
    //     .unshift(doc.into_router("/api-doc/openapi.json"))
    //     .unshift(
    //         Scalar::new("/api-doc/openapi.json")
    //             .title("Palpo - Scalar")
    //             .into_router("/scalar"),
    //     )
    //     .unshift(SwaggerUi::new("/api-doc/openapi.json").into_router("/swagger-ui"));
    let catcher = Catcher::default().hoop(hoops::catch_status_error);
    let service = Service::new(router)
        .catcher(catcher)
        .hoop(hoops::default_accept_json)
        .hoop(Logger::new())
        .hoop(
            Cors::new()
                .allow_origin(cors::Any)
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(AllowHeaders::list([
                    salvo::http::header::ACCEPT,
                    salvo::http::header::CONTENT_TYPE,
                    salvo::http::header::AUTHORIZATION,
                    salvo::http::header::RANGE,
                ]))
                .max_age(Duration::from_secs(86400))
                .into_handler(),
        )
        .hoop(hoops::remove_json_utf8);
    let service = if conf.compression.is_enabled() {
        let mut compression = Compression::new();
        if conf.compression.enable_brotli {
            compression = compression.enable_zstd(CompressionLevel::Fastest);
        }
        if conf.compression.enable_zstd {
            compression = compression.enable_zstd(CompressionLevel::Fastest);
        }
        if conf.compression.enable_gzip {
            compression = compression.enable_gzip(CompressionLevel::Fastest);
        }
        service.hoop(compression)
    } else {
        service
    };
    let _ = crate::data::user::unset_all_presences();

    salvo::http::request::set_global_secure_max_size(8 * 1024 * 1024);
    let conf = crate::config::get();
    let mut acceptors = vec![];
    for listener_conf in &conf.listeners {
        if let Some(tls_conf) = listener_conf.enabled_tls() {
            tracing::info!("Listening on: {} with TLS", listener_conf.address);
            let acceptor = TcpListener::new(&listener_conf.address)
                .rustls(RustlsConfig::new(
                    Keycert::new()
                        .cert_from_path(&tls_conf.cert)?
                        .key_from_path(&tls_conf.key)?,
                ))
                .bind()
                .await
                .into_boxed();
            acceptors.push(acceptor);
        } else {
            tracing::info!("Listening on: {}", listener_conf.address);
            let acceptor = TcpListener::new(&listener_conf.address)
                .bind()
                .await
                .into_boxed();
            acceptors.push(acceptor);
        }
    }

    Server::new(DynTcpAcceptors::new(acceptors))
        .serve(service)
        .instrument(tracing::info_span!("server.serve"))
        .await;
    Ok(())
}
