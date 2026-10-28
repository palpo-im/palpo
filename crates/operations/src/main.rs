use std::path::Path;

use palpo_operations::api::{App, router};
use palpo_operations::matrix::Matrix;
use palpo_operations::store::Store;
use salvo::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = std::env::var("PALPO_SERVER_NAME")?;
    let matrix = Matrix::new(&std::env::var("PALPO_URL")?, server.try_into()?)?;
    let database = std::env::var("PALPO_ADMIN_DATABASE")?;
    let store = Store::open(Path::new(&database))?;
    if std::env::args().len() != 1 {
        return Err("usage: palpo-operations".into());
    }
    let public = std::env::var("PUBLIC_ORIGIN")?;
    let mut app = App::new(matrix, store, &public, 900000)?;
    let address =
        std::env::var("PALPO_OPERATIONS_LISTEN").unwrap_or_else(|_| "127.0.0.1:8091".into());
    if let Ok(path) = std::env::var("PALPO_ACCOUNT_CONFIG") {
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err("account configuration must be a bounded regular file".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("account configuration must be private (0600)".into());
            }
        }
        app = app.with_accounts(serde_json::from_slice(&std::fs::read(path)?)?)?;
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
    let accounts = palpo_operations::accounts::start(app.clone());
    server.serve(router(app)).await;
    accounts.abort();
    let _ = accounts.await;
    Ok(())
}
