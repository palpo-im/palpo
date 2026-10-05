use std::path::Path;

use palpo_operations::api::{App, router};
use palpo_operations::matrix::Matrix;
use palpo_operations::store::Store;
use palpo_operations::workflow::{AuthoritySnapshot, Workflows};
use salvo::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = std::env::var("PALPO_SERVER_NAME")?;
    let matrix = Matrix::new(&std::env::var("PALPO_URL")?, server.try_into()?)?;
    let database = std::env::var("PALPO_ADMIN_DATABASE")?;
    let mut store = Store::open(Path::new(&database))?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !args.is_empty() {
        if args.len() != 2 || args[0] != "import-authority" {
            return Err(
                "usage: palpo-operations [import-authority <reviewed-snapshot.json>]".into(),
            );
        }
        let raw = std::fs::read(&args[1])?;
        if raw.len() > 1024 * 1024 {
            return Err("authority snapshot exceeds 1 MiB".into());
        }
        let snapshot: AuthoritySnapshot = serde_json::from_slice(&raw)?;
        store.bind(matrix.server().as_str(), &matrix.origin())?;
        store.transaction(|state| {
            let mut workflows = Workflows::load(state)?;
            workflows.import_authority(snapshot, matrix.server())?;
            workflows.save(state)
        })?;
        println!(
            "Reviewed authority snapshot imported. Existing decisions and delivery records retained."
        );
        return Ok(());
    }
    let public = std::env::var("PUBLIC_ORIGIN")?;
    let mut app = App::new(matrix, store, &public, 900000)?;
    if let Ok(admin) = std::env::var("PALPO_ASSOCIATION_ADMIN") {
        app = app.with_association_admin(admin.try_into()?)?;
    }
    match (
        std::env::var("PALPO_TRANSPORT_ORIGIN"),
        std::env::var("PALPO_RELAY_ORIGIN"),
    ) {
        (Ok(transport), Ok(relay)) => {
            app = app.with_transport(&transport, &relay)?;
        }
        (Err(_), Err(_)) => {}
        _ => {
            return Err(
                "PALPO_TRANSPORT_ORIGIN and PALPO_RELAY_ORIGIN must be configured together".into(),
            );
        }
    }
    let address =
        std::env::var("PALPO_OPERATIONS_LISTEN").unwrap_or_else(|_| "127.0.0.1:8091".into());
    if let Ok(path) = std::env::var("PALPO_ACTIONS_BOT_TOKEN_FILE") {
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.len() > 8192 {
            return Err("notification token must be a bounded regular file".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("notification token file must be private (0600)".into());
            }
        }
        app = app.with_notifications(palpo_operations::notifications::Configuration {
            bot: std::env::var("PALPO_ACTIONS_BOT_MXID")?.try_into()?,
            token: std::fs::read_to_string(path)?.trim().to_owned(),
            public_origin: std::env::var("PALPO_ACTIONS_PUBLIC_ORIGIN")?,
            quiet_start: std::env::var("PALPO_ACTIONS_QUIET_START_UTC")
                .unwrap_or_else(|_| "0".into())
                .parse()?,
            quiet_end: std::env::var("PALPO_ACTIONS_QUIET_END_UTC")
                .unwrap_or_else(|_| "0".into())
                .parse()?,
        })?;
    }
    let acceptor = TcpListener::new(address).try_bind().await?;
    let server = Server::new(acceptor);
    let handle = server.handle();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut terminate) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            } else {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        handle.stop_graceful(Some(std::time::Duration::from_secs(10)));
    });
    let notifications = palpo_operations::notifications::start(app.clone());
    server.serve(router(app)).await;
    notifications.abort();
    let _ = notifications.await;
    Ok(())
}
