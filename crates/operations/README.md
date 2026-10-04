# Rust Operations service

`palpo-operations` begins the backend JavaScript migration required by
[Rinx ADR 0011](https://github.com/hagency-org/Rinx/blob/main/docs/adr/0011-hagency-server-engagements.md).
It is a Rust executable and a mountable Salvo router. It never invokes Node,
serves JavaScript, or proxies business decisions to the old service. Rinx's
native host remains Rust and the mini-app presentation remains OctoScript.

This is an integration slice, **not a production replacement for `web-admin`**.
The legacy server stays available until the remaining routes and workers have
equivalent Rust implementations and the cutover gates below pass.

## Implemented

| Existing backend | Rust implementation | Boundary |
| --- | --- | --- |
| `lib/store.mjs` | `store.rs` | Existing SQLite `state` table, complete JSON preservation, process lock, atomic transactions |
| `lib/miniapp.mjs` | `api.rs`, `matrix.rs` | Borrowed Matrix login, scoped in-memory sessions, revalidation, disconnect |
| `lib/inbox.mjs` | `workflow.rs` | Typed project/agent requests, coordinator decisions, visibility, seen/snooze, durable receipts |
| `lib/action-notifications.mjs` | `workflow.rs` | Durable notification intents only; Matrix delivery is still pending |
| `lib/workflow.mjs`, `lib/service.mjs` | `palpo-hagency-contract` and decision outbox | Current authority checks and commands; Matrix/Hagency execution is still pending |

An authenticated project owner can submit a project against an engagement's
allocated resources. Its designated coordinator can approve. For an agent on a
ready project, the coordinator or Hagency resource owner can approve. A Matrix
server administrator has no implicit project or agent approval authority. This
follows ADR 0011; it deliberately supersedes the previous server-admin project
approval policy. Self-approval requires an explicit engagement policy.

Approval writes the decision, command receipt, Hagency outbox entry and
notification intents in one SQLite transaction. Competing decisions cannot both
win. Repeating a command with changed content conflicts; an exact retry does not
enqueue a second command. Current authority is still required for a mutation
retry; retrieving the action returns its latest stored result. Seen/snoozed
notifications do not complete the action.

**Approved is not live.** Execution remains `pending`; this service has no
Hagency worker, ledger reservation or provisioning receipt yet. It cannot claim
capacity is reserved, deduct tokens, create rooms, announce a live agent, or
deliver Matrix notifications. These capabilities are explicitly false in the
session feature response.

## Run in an isolated development environment

```sh
cargo build --locked -p palpo-operations
export PALPO_SERVER_NAME=example.test
export PALPO_URL=https://matrix.example.test
export PALPO_ADMIN_DATABASE=/tmp/palpo-operations-dev/admin.sqlite
export PUBLIC_ORIGIN=http://127.0.0.1:8091
export PALPO_OPERATIONS_LISTEN=127.0.0.1:8091
./target/debug/palpo-operations
```

`GET /healthz` reports process health, not Matrix/Hagency connection proof.
Matrix is contacted for actual sessions/calls. `PALPO_URL` is a fixed HTTPS
origin, with HTTP permitted on explicit loopback hosts for development. Public
exposure uses TLS termination and the exact configured `PUBLIC_ORIGIN` Host.

`POST /_palpo/miniapp/v1/session` accepts the existing `appId`, `bundleDigest`,
`services` envelope and a Matrix bearer token. `call` uses the returned mini-app
bearer and `{ "service": "...", "args": {} }`; `disconnect` revokes only that
mini-app session. Sessions expire within 15 minutes, are memory-only, and check
Matrix identity again on every call, including after waiting for a mutation.

Only these services can be granted:

- `palpo.intent.new`, `palpo.session.open`, `palpo.session.disconnect`
- `palpo.inbox.list`, `palpo.inbox.get`, `palpo.inbox.submit`
- `palpo.inbox.decide`, `palpo.inbox.seen`, `palpo.inbox.snooze`

The feature `rustWorkflowRequests: 1` identifies the new request schema.
Submission takes `{ "kind": "project" | "agent", "request": <typed request> }`.
Approval takes `{ "id": <action>, "decision": "approve", "command": <typed approval> }`.
Rejection takes `id`, `decision: "reject"`, `expectedRevision`, `commandId` and
`reason`. See [HTTP integration tests](tests/workflows.rs) for complete payloads.
All command identities, request contents and current registration/delegation/
project revisions are checked against server state, not trusted from the caller.

The existing complete Rinx bundle requests more services and uses the legacy
flat submission shape. It **cannot be redirected to this server unchanged**.
Unknown capabilities and unsupported routes fail rather than silently executing
the old flow. A Rinx adapter must gate the new protocol on this feature.

## Authority and migration

The service requires a trusted projection of verified engagements, funded
resources, eligible managers and ready projects. At this stage it is imported
through an explicit **offline operator command**, not from an app request:

```sh
./target/debug/palpo-operations import-authority /path/to/reviewed-authority.json
```

The JSON contains `engagements`, `resources` and `projects` maps. Use the
`authority` fixture in [tests/workflows.rs](tests/workflows.rs) as a schema
example, not as connection-proof evidence. The import validates local identities,
map IDs, engagement bindings and monotonic revisions. It rejects authority removal
that could reset revision history: retain revoked engagements/project tombstones
and zero-capacity withdrawn resources. A replaced coordinator requires a new
delegation revision; a revoked engagement requires a new registration generation
before reactivation. The future authenticated Hagency projection worker must
verify provenance and use these same monotonic checks.

Migration checks use a **copy** of an existing database. Both executables must
never own the same database concurrently. The Rust executable honors the Node
`${PALPO_ADMIN_DATABASE}.lock` convention, refuses existing locks and database
symlinks, and releases its lock on graceful shutdown. It never steals stale
locks; recovery requires confirming that the previous process has stopped.
New files use private permissions on Unix.

All legacy JSON (including credentials and unknown extension fields) and the
`fleet_delivery` table are preserved. Rust data lives in `rustWorkflows`.
Preservation is not a semantic migration: old Inbox records are **not converted**
into new coordinator requests, and old delivery rows are **not consumed** by
this service. Neither legacy requests nor hostnames can establish new role
bindings. Explicit reconciliation is required before production cutover.

## Remaining cutover gates

1. Rust engagement association, designated-admin approval/profile export,
   coordinator delegation and real connection proof; independent registrations
   for multiple engagements, including those with the same Matrix hostname.
2. Rust equivalents for fleet/admin routes, durable outbound transport,
   retry/reconciliation, account operations and Matrix notification workers.
3. Hagency command authentication, hierarchy-aware transactional reservations,
   provisioning and receipts. One coordinator decision must suffice; the Hagency
   portal must not add a second approval. Agent/top-up approvals must not silently
   enlarge their engagement or parent pool.
4. Persist and expose every delivered approved agent, including failed or pending
   provisioning, in Hagency and the owner's list; verify chat readiness separately.
   Usage must include freshness and unknown states. Job summaries remain future work.
5. Rinx Rust-host/OctoScript schema and role screens, real Makepad instrumentation,
   then end-to-end testing with isolated accounts before a deployment cutover.
6. Reconcile old requests/roles/queues; remove migrated `.mjs` backend modules and
   the Node production image only after all supported routes and workers pass.

## Validation

```sh
cargo test --locked -p palpo-hagency-contract -p palpo-operations
cargo clippy --locked -p palpo-hagency-contract -p palpo-operations --all-targets -- -D warnings
cargo +nightly fmt -p palpo-hagency-contract -p palpo-operations -- --check
cargo build --locked -p palpo-operations
python3 crates/operations/tests/node_migration.py --node node --binary target/debug/palpo-operations
```

Node 24 is required only by the cross-language migration test. It creates the
database using the actual existing Node `Store` and `Service`, invokes the Rust
executable, then reopens the result with Node. It checks preserved state and
delivery rows, process-lock exclusion, actual HTTP startup and graceful SIGTERM
cleanup. Rust integration tests exercise a real loopback Matrix stub, permission
denials, atomic decision conflicts, restart replay, revocation and migration.
These are backend tests, not Makepad or live Hagency end-to-end validation.
