#![allow(dead_code, missing_docs)]
#![recursion_limit = "512"]
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
pub mod storage;
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
pub use jsonwebtoken as jwt;
pub use palpo_core as core;
pub use palpo_data as data;
pub use palpo_server_macros as macros;
use salvo::catcher::Catcher;
use salvo::compression::{Compression, CompressionLevel};
use salvo::conn::rustls::{Keycert, RustlsConfig};
use salvo::conn::tcp::DynTcpAcceptors;
use salvo::cors::{AllowHeaders, AllowOrigin, Cors, CorsHandler};
use salvo::http::Method;
use salvo::logging::Logger;
use salvo::prelude::*;
use tracing_futures::Instrument;

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

fn cors_handler(allowed_origins: &[String]) -> CorsHandler {
    let allow_origin = if allowed_origins.is_empty() {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            allowed_origins
                .iter()
                .map(|origin| origin.parse().expect("allowed CORS origin was validated")),
        )
    };

    Cors::new()
        .allow_origin(allow_origin)
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
        .into_handler()
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

    /// Admin command to execute during startup. May be specified repeatedly.
    #[arg(long, value_name = "COMMAND")]
    pub(crate) execute: Vec<String>,

    #[arg(long, short, num_args(1), default_value_t = true)]
    pub(crate) server: bool,
}

const TOKIO_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(TOKIO_WORKER_STACK_SIZE)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Install rustls CryptoProvider for TLS (federation + outbound HTTPS)
    let _ = rustls::crypto::ring::default_provider().install_default();

    if dotenvy::from_filename(".env.local").is_err() {
        tracing::debug!(".env.local file is not found");
    }
    if let Err(e) = dotenv() {
        tracing::info!("dotenv not loaded: {:?}", e);
    }

    let args = Args::parse();

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
    crate::storage::init(&conf.storage).expect("Failed to initialize storage backend");
    // Force-load appservice registrations during startup so database rows
    // are up-to-date with the configured registration directory.
    let _ = crate::appservices().await;

    // Startup admin commands can enqueue federation, appservice, or push work,
    // so prepare the wakeup queue for those durable requests before executing
    // them. The network worker starts only after the no-server exit below.
    let sending_guard = crate::sending::guard::init();

    let console = args.console || conf.admin.console_automatic;
    let admin = crate::admin::init(args.execute).await?;

    if console {
        tracing::info!("starting admin console...");
    }

    if !args.server {
        if console {
            admin.run(true).await?;
            tracing::info!("admin console stopped");
        } else {
            tracing::info!("server is not started, exiting...");
        }
        return Ok(());
    }

    sending_guard.start();

    tokio::spawn(async move {
        if let Err(error) = admin.run(console).await {
            tracing::error!(?error, "admin command processor stopped");
            if console {
                std::process::exit(1);
            }
        } else if console {
            tracing::info!("admin console stopped, shutting down...");
            std::process::exit(0);
        } else {
            tracing::error!("admin command processor stopped");
        }
    });

    // MSC2444: periodically renew our outbound room peeks and drop lapsed inbound
    // peekers.
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            crate::federation::peek::run_maintenance().await;
        }
    });

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
        .hoop(cors_handler(&conf.allowed_origins))
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
    // In a clustered deployment, do NOT clear all presence on startup.
    // Other instances may still be serving users who are online.
    // Stale presence will be cleaned up by the presence timeout logic.
    // let _ = crate::data::user::unset_all_presences();

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

#[cfg(test)]
mod tests {
    use salvo::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ORIGIN};
    use salvo::test::TestClient;

    use super::*;

    #[test]
    fn execute_cli_argument_can_be_repeated() {
        let args = Args::try_parse_from([
            "palpo",
            "--execute",
            "server show-version",
            "--execute",
            "user list",
        ])
        .unwrap();

        assert_eq!(args.execute, ["server show-version", "user list"]);
    }

    #[handler]
    async fn test_handler(res: &mut Response) {
        res.render(Text::Plain("ok"));
    }

    fn service(allowed_origins: &[String]) -> Service {
        Service::new(Router::new().get(test_handler)).hoop(cors_handler(allowed_origins))
    }

    #[tokio::test]
    async fn cors_allows_any_origin_when_list_is_empty() {
        let response = TestClient::get("http://localhost/")
            .add_header(ORIGIN, "https://client.example", true)
            .send(&service(&[]))
            .await;

        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "*"
        );
    }

    #[tokio::test]
    async fn cors_only_echoes_configured_origins() {
        let allowed_origins = vec!["https://client.example".to_owned()];
        let service = service(&allowed_origins);

        let allowed = TestClient::get("http://localhost/")
            .add_header(ORIGIN, "https://client.example", true)
            .send(&service)
            .await;
        assert_eq!(
            allowed.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
            "https://client.example"
        );

        let rejected = TestClient::get("http://localhost/")
            .add_header(ORIGIN, "https://other.example", true)
            .send(&service)
            .await;
        assert!(
            rejected
                .headers()
                .get(ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }
}
