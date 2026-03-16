use anyhow::Result;
use palpo_admin_server::{
    handlers::{webui_admin, server_control, matrix_admin},
    MigrationRunner, MigrationService, SessionManager, WebUIAuthService,
    MatrixAdminCreationService, AuthService, PalpoClient, ServerControlAPI,
};
use palpo_data::DbConfig;
use salvo::prelude::*;
use salvo::cors::{self, AllowHeaders, Cors};
use salvo::http::Method;
use std::env;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();

    info!("Starting Palpo Admin Server...");

    // Get database URL from environment or use default
    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://palpo:password@localhost/palpo".to_string());

    // Create database configuration
    let db_config = DbConfig {
        url: database_url,
        pool_size: 10,
        min_idle: Some(2),
        tcp_timeout: 10000,
        connection_timeout: 30000,
        statement_timeout: 30000,
        helper_threads: 10,
        enforce_tls: false,
    };

    // Initialize palpo-data (database connection and schema)
    info!("Initializing database...");
    palpo_data::init(&db_config);
    
    // Run database migrations
    info!("Running database migrations...");
    palpo_data::migrate();

    info!("Database initialized successfully");

    // Initialize admin-specific services
    let db_pool = palpo_data::DIESEL_POOL
        .get()
        .expect("Database pool should be initialized")
        .clone();
    let migration_runner = MigrationRunner::new(db_pool.clone());
    let auth_service = Arc::new(WebUIAuthService::new(db_pool.clone()));
    let session_manager = Arc::new(SessionManager::new());
    let migration_service = Arc::new(MigrationService::new(WebUIAuthService::new(db_pool.clone())));
    let server_control = Arc::new(ServerControlAPI::new());

    // Initialize Matrix admin services
    let homeserver_url = env::var("HOMESERVER_URL")
        .unwrap_or_else(|_| "http://localhost:8008".to_string());
    let matrix_creation_service = Arc::new(MatrixAdminCreationService::new(homeserver_url.clone()));
    let matrix_auth_service = Arc::new(AuthService::new());

    // Initialize PalpoClient for admin API calls
    let palpo_base_url = env::var("PALPO_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8008".to_string());
    let palpo_admin_username = env::var("PALPO_ADMIN_USERNAME")
        .unwrap_or_else(|_| "admin".to_string());
    let palpo_admin_password = env::var("PALPO_ADMIN_PASSWORD")
        .unwrap_or_else(|_| "password".to_string());
    
    let palpo_client = Arc::new(PalpoClient::new(
        palpo_base_url,
        palpo_admin_username,
        palpo_admin_password,
    ));
    
    // Login to Palpo to get access token
    info!("Logging in to Palpo...");
    match palpo_client.login().await {
        Ok(_) => {
            info!("Successfully logged in to Palpo");
        }
        Err(e) => {
            tracing::warn!("Failed to login to Palpo: {}. Server will continue without Palpo connection.", e);
            tracing::warn!("Palpo service can be started later via the UI");
        }
    }

    // Run admin-specific migrations
    info!("Running admin migrations...");
    if let Err(e) = migration_runner.run_migrations() {
        tracing::error!("Failed to run admin migrations: {}", e);
        return Err(e.into());
    }
    info!("Admin migrations completed successfully");

    // Create shared application state
    let app_state = webui_admin::AppState {
        auth_service,
        session_manager: session_manager.clone(),
        migration_service,
    };

    // Initialize global state
    webui_admin::init_app_state(app_state);

    // Create server control state
    let server_control_state = server_control::ServerControlState {
        server_control,
    };

    // Initialize server control state
    server_control::init_server_control_state(server_control_state);

    // Create Matrix admin state
    let matrix_admin_state = matrix_admin::MatrixAdminState {
        creation_service: matrix_creation_service,
        auth_service: matrix_auth_service,
        homeserver_url,
    };

    // Initialize Matrix admin state
    matrix_admin::init_matrix_admin_state(matrix_admin_state);

    // Configure CORS - allow any origin for development
    let cors = Cors::new()
        .allow_origin(cors::Any)
        .allow_methods(vec![Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::PATCH, Method::OPTIONS])
        .allow_headers(AllowHeaders::list([
            salvo::http::header::CONTENT_TYPE,
            salvo::http::header::AUTHORIZATION,
        ]));

    // Create router with Web UI Admin endpoints
    let router = Router::new()
        .push(
            Router::with_path("/api/v1/admin/webui-admin")
                .push(Router::with_path("/status").get(webui_admin::status))
                .push(Router::with_path("/setup").post(webui_admin::setup))
                .push(Router::with_path("/login").post(webui_admin::login))
                .push(Router::with_path("/change-password").post(webui_admin::change_password))
                .push(Router::with_path("/logout").post(webui_admin::logout))
                .push(Router::with_path("/migrate").post(webui_admin::migrate)),
        )
        .push(
            Router::with_path("/api/v1/admin/server")
                .push(Router::with_path("/config")
                    .get(palpo_admin_server::handlers::server_config::get_config)
                    .post(palpo_admin_server::handlers::server_config::save_config)
                    .push(Router::with_path("/validate")
                        .post(palpo_admin_server::handlers::server_config::validate_config)
                    )
                )
                .push(Router::with_path("/status")
                    .get(server_control::get_status)
                )
                .push(Router::with_path("/start")
                    .post(server_control::start_server)
                )
                .push(Router::with_path("/stop")
                    .post(server_control::stop_server)
                )
                .push(Router::with_path("/restart")
                    .post(server_control::restart_server)
                )
        )
        .push(
            Router::with_path("/api/v1/config")
                .push(Router::with_path("/form")
                    .get(palpo_admin_server::handlers::server_config::get_config_form)
                    .post(palpo_admin_server::handlers::server_config::save_config_form)
                )
                .push(Router::with_path("/metadata")
                    .get(palpo_admin_server::handlers::server_config::get_config_metadata)
                )
                .push(Router::with_path("/reset")
                    .post(palpo_admin_server::handlers::server_config::reset_config_handler)
                )
                .push(Router::with_path("/reload")
                    .post(palpo_admin_server::handlers::server_config::reload_config_handler)
                )
                .push(Router::with_path("/search")
                    .get(palpo_admin_server::handlers::server_config::search_config)
                )
                .push(Router::with_path("/toml")
                    .get(palpo_admin_server::handlers::server_config::get_config_toml)
                    .post(palpo_admin_server::handlers::server_config::save_config_toml)
                    .push(Router::with_path("/validate")
                        .post(palpo_admin_server::handlers::server_config::validate_toml)
                    )
                    .push(Router::with_path("/parse")
                        .post(palpo_admin_server::handlers::server_config::parse_toml)
                    )
                )
                .push(Router::with_path("/export")
                    .post(palpo_admin_server::handlers::server_config::export_config)
                )
                .push(Router::with_path("/import")
                    .post(palpo_admin_server::handlers::server_config::import_config)
                )
        )
        .push(
            Router::with_path("/api/v1/server")
                .push(Router::with_path("/version")
                    .get(palpo_admin_server::handlers::server_config::get_server_version)
                )
        )
        .push(
            Router::with_path("/api/v1/admin/health")
                .push(Router::with_path("/status")
                    .get(palpo_admin_server::handlers::server_status::get_health)
                )
                .push(Router::with_path("/metrics")
                    .get(palpo_admin_server::handlers::server_status::get_metrics)
                )
                .push(Router::with_path("/version")
                    .get(palpo_admin_server::handlers::server_status::get_version)
                )
        )
        .push(
            Router::with_path("/api/v1/admin/matrix-admin")
                .push(Router::with_path("/create")
                    .post(matrix_admin::create_matrix_admin)
                )
                .push(Router::with_path("/login")
                    .post(matrix_admin::login_matrix_admin)
                )
                .push(Router::with_path("/change-password")
                    .post(matrix_admin::change_matrix_admin_password)
                )
        )
        .push(Router::with_path("/health").get(health_check));

    // Create acceptor and bind to port 8081
    let acceptor = TcpListener::new("0.0.0.0:8081").bind().await;
    
    info!("Admin Server listening on http://0.0.0.0:8081");
    
    // Create service with CORS middleware
    let service = Service::new(router).hoop(cors.into_handler());
    
    // Start server
    Server::new(acceptor).serve(service).await;

    Ok(())
}

/// Health check endpoint
#[handler]
async fn health_check() -> &'static str {
    "OK"
}
