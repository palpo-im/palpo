# Embedding Palpo in a Salvo server

The `palpo` package exposes a library alongside its unchanged CLI binary.
`MatrixServer::initialize(ServerConfig)` validates and installs configuration,
runs database migrations, initializes media and App Services, and starts the
sending, admin-room, delayed-event and maintenance workers. It does not parse
CLI arguments, install tracing, read dotenv files, or bind HTTP listeners.

```rust,no_run
use palpo::{MatrixServer, config::ServerConfig};
use salvo::prelude::*;

async fn embed(mut config: ServerConfig, host_routes: Router) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    config.well_known.client = Some("https://matrix.example.com".to_owned());
    let matrix = MatrixServer::initialize(config).await?;
    let router = host_routes.push(matrix.router());
    let acceptor = TcpListener::new("127.0.0.1:8088").bind().await;
    Server::new(acceptor).serve(matrix.service(router)).await;
    Ok(())
}
```

Embedded initialization requires `well_known.client` to be an absolute HTTP(S)
URL with a host. Set it to the public base URL that Matrix clients can reach,
including any port or path prefix, without credentials, a query, or a fragment.
Palpo's `listeners` are unused in embedding
mode and cannot determine the host's TLS scheme or public address. Configure
`well_known.server` as well if federation uses an address or port different from
the default `server_name:443`.

`MatrixServer::router()` includes `/_matrix`, `/_palpo` (including the existing
admin authentication), and `/.well-known/matrix`. It omits Palpo's homepage,
health routes, and `./static` wildcard. The host can serve its own homepage and
assets without competing with a catch-all. `routing::root()` retains these
routes for standalone Palpo.

`MatrixServer::service(router)` applies the same error catcher, JSON handling,
CORS, logger, and compression as the standalone binary. The host owns Tokio,
tracing, listeners, TLS termination, and graceful HTTP shutdown. Attach
`palpo::logging::capture_layer()` to the host tracing subscriber to include
command-scoped diagnostic logs in admin-room replies; capture state is initialized
without replacing the host subscriber. Use Tokio
worker stacks of 8 MiB, as the CLI does, for deep Matrix event processing.

Matrix routes set their own 8 MiB parsing limit, matching standalone Palpo,
without changing the host's global body limit or limits on other routes.
Compression only enables the algorithms selected in `compression`.

Database connectivity and migrations, storage, signing keys, and App Service
registration are prepared before publishing global configuration and starting
workers. Their failures return an initialization error and leave Palpo's globals
unset, so the host can correct configuration or restore connectivity and retry.
Synchronous database preparation runs on Tokio's blocking pool. Initializers are
serialized, so concurrent successful attempts cannot replace each other's state.
Migrations and registration upserts may already have committed to PostgreSQL
before a later preparation failure; retrying uses the same idempotent operations.

Palpo still uses process-wide singleton configuration, storage and database
pools. After global state is published, only one `MatrixServer` can be initialized
in a process; a second call returns an error. A failure executing configured
startup admin commands happens after publication and requires a fresh process
to retry. Dropping the handle does not reset singleton state or stop
workers. Workers stop when the host runtime shuts down. An interactive automatic
admin console is rejected for embedding; admin-room commands remain active.

The opt-in startup regression test requires an **empty, dedicated** PostgreSQL
database and covers migration and App Service failures, retry, concurrent
initializers, and mounted discovery:

```sh
PALPO_EMBEDDING_TEST_DATABASE_URL=postgres://user:password@localhost/palpo_embedding_test \
  cargo test -p palpo --lib embedding_startup_retries_postgres -- --ignored --nocapture
```
