# Optional account approval service

`palpo-operations` is a separate Rust service for ordinary Matrix account signup
approvals. It retains the public request/receipt API, private Matrix approval
room, registration worker, and scoped native administration sessions. It does
not grant server administration privileges to applicants.

Hagency fleet installation, pairing, project/coordinator decisions, Agent
lifecycle, resource catalogs, budgets, Inbox, runtime status and leased relay
transport have been removed. The shared Hagency contract and legacy Node
migration fixtures are also removed. The standalone Palpo homeserver does not
mount this service automatically.

## Run

Build with `cargo build --locked -p palpo-operations`, then configure:

```sh
export PALPO_SERVER_NAME=example.test
export PALPO_URL=https://matrix.example.test
export PALPO_ADMIN_DATABASE=/private/palpo-accounts/admin.sqlite
export PUBLIC_ORIGIN=https://accounts.example.test
export PALPO_OPERATIONS_LISTEN=127.0.0.1:8091
export PALPO_ACCOUNT_CONFIG=/private/palpo-accounts/accounts.json
```

Run the built `palpo-operations` executable. These environment names and the
executable name are retained for existing account-approval deployments. The
worker is enabled only when `PALPO_ACCOUNT_CONFIG` is configured. `GET /healthz`
reports process health; `GET /api/account-access` reports live signup readiness.

The account configuration is a private regular JSON file (0600 on Unix, at most
64 KiB) with `botMxid`, `botToken`, `adminToken`, `approvers`, `passwordKey` and
`registrationToken`. `passwordKey` is 32 random bytes encoded as lowercase hex.
Use a dedicated local notification bot and explicitly listed local human
administrators. Configure a Matrix registration-token flow; the worker completes
that UIAA flow through the ordinary registration endpoint. The service does not
bypass delegated authentication or the homeserver's registration policy.

TLS termination must preserve the configured `PUBLIC_ORIGIN` Host. Public POSTs
require JSON and same-origin browser requests. Rate limits use the socket peer,
not caller-supplied forwarding headers. Store files contain credentials or
sealed passwords and must remain private.

## Signup API and decisions

- `GET /api/account-access` exposes configuration/readiness without credentials.
- `POST /api/account-requests` accepts `id` (32 lowercase hex characters),
  `receipt` (64 lowercase hex characters), `username`, `password`, `displayName`
  and `reason`. Keep the same ID and receipt when retrying a lost reply.
- `POST /api/account-requests/status` accepts only `id` and `receipt` and returns
  the bounded public request state.

The worker rechecks current bot/admin authority and private-room membership.
It sends `org.octos.approval_request` metadata in ordinary Matrix messages;
listed approvers reply with the matching request ID, digest, source event and
`org.octos.approval_response`. Native navigation resolves that original card.
Passwords are sealed with AES-GCM, removed on terminal outcomes, and never
included in approval messages or status responses. Registration retries use the
original random device as ownership proof after a lost response; an existing
unrelated account is never adopted or overwritten. Session access tokens from
registration are never persisted.

`POST /_palpo/miniapp/v1/session`, `/call` and `/disconnect` retain the existing
`appId: im.palpo.operations`, SHA-256 `bundleDigest` and service-grant envelope.
Only these services remain:

- `palpo.intent.new`
- `palpo.session.open`, `palpo.session.disconnect`
- `palpo.accounts.list`, `palpo.accounts.open`

These are private application interfaces. A native host supplies its Matrix
bearer to create an in-memory session lasting at most 15 minutes. Every call
revalidates the underlying identity; account reads/navigation require current
administrator authority and the exact service grant. Disconnecting the app
session does not log out the Matrix account. Session routes reject browser
cookies and Origin headers. Removed workflow capabilities and association/relay
routes are unavailable; clients must request only the supported service list.

## Existing databases and deployments

Stop the old service before opening its database. The existing lock convention,
server binding, signup IDs, password format, receipts, account worker cursor,
private-room binding and previously installed writer fences remain compatible.
The service preserves unknown historical JSON fields and existing SQLite tables;
it does not execute retired work, import allocations, remove data or transfer
Hagency authority. Keep an untouched backup and the previous executable if any
retired application is still needed. Never run two writers against one database.

See the [retirement guide](../../docs/HAGENCY_MIGRATION.md) for application
ownership and deployment boundaries. This source change performs no deployment,
data migration or cutover to a replacement Hagency service.

## Validation

```sh
cargo test --locked -p palpo-operations
cargo clippy --locked -p palpo-operations --all-targets -- -D warnings
cargo +nightly fmt -p palpo-operations -- --check
```

The HTTP tests use a controlled Matrix peer and exercise actual Rust routing,
SQLite state, approval authorization, registration recovery, session grants and
removed routes. They do not claim a production deployment or cross-project E2E
acceptance. No Node runtime is required for these tests or for the service.
