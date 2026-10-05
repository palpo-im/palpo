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
| `lib/miniapp.mjs` project/agent reads | `views.rs` | Role-scoped pagination, distinct approval/execution state, lower-bound token observations with freshness |
| `lib/inbox.mjs` | `workflow.rs` | Typed project/agent/top-up requests, coordinator decisions, visibility, seen/snooze, durable receipts |
| `lib/action-notifications.mjs` | `notifications.rs` | Durable private My Actions rooms, idempotent delivery, reminders, quiet hours and pinned Inbox board |
| `lib/workflow.mjs` creation forms | `creation.rs`, `rooms.rs` | Funded catalog, owner room preparation, frozen project/agent submissions and lost-reply reconciliation |
| `lib/outbound.mjs` | `outbound.rs`, `machine.rs`, `updates.rs` | Existing SQL lease queue, relay/poll/ACK/update routes, generations, probe receipts and bounded runtime observations |
| `lib/workflow.mjs`, `lib/service.mjs` | contract, decision outbox, `updates.rs` | Immutable definitions delivered to Hagency; scoped resource/project projections and execution receipts |

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

Form approvals expire with the owner's delegation, not after a ten-minute UI
window. A temporarily offline Hagency can execute the original decision while
the exact delegation, registration and project bindings remain valid. Revoked,
changed or expired authority still refuses execution.

The existing OctoScript decision form (`id`, `expectedRevision`, `decision`,
`commandId`, `reason`) is also accepted. Rust builds its authority envelope from
the authenticated account and stored request; the app cannot select its actor,
delegation, generation or grant. Its stable intent digest makes exact retries
idempotent even after a later action revision. Agent top-ups bind the current
allocation and cannot claim that new tokens are available before Hagency commits.

**Approved is not live.** Without a definition it remains `pending`. With an
immutable definition and a capable installed Hagency engagement, the decision
and leased work item commit together. A custody ACK still leaves it pending.
An authenticated execution receipt advances it to `provisioning`; current
runtime and Matrix-membership observations are required for `ready`.
Duplicate receipts do not execute another allocation. Old source observations
stay stale even when delivered now. Token usage stays unknown when unreported;
measurement time, evidence, completeness and quota pause are preserved. App
Service registration and machine credential generations are checked separately.
Profile installation and association approval remain pending.

## Run in an isolated development environment

```sh
cargo build --locked -p palpo-operations
export PALPO_SERVER_NAME=example.test
export PALPO_URL=https://matrix.example.test
export PALPO_ADMIN_DATABASE=/tmp/palpo-operations-dev/admin.sqlite
export PUBLIC_ORIGIN=http://127.0.0.1:8091
export PALPO_OPERATIONS_LISTEN=127.0.0.1:8091
# Optional; configure both to enable machine routes:
export PALPO_TRANSPORT_ORIGIN=https://operations.example.test
export PALPO_RELAY_ORIGIN=http://127.0.0.1:8092
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
- `palpo.projects.list`, `palpo.requests.list`
- `palpo.catalog.list`, `palpo.requests.create`

Project and agent reads take optional `offset` and `limit` (1–100, default 50).
They return only records belonging to the caller or its current engagement
authority, with no implicit server-admin access. The agent list includes every
visible request, including approved requests awaiting Hagency execution. Missing
allocation or usage stays null. Usage is an attributed lower bound, never an
exact remaining balance; old samples retain their value and are marked stale.
Machine credential rotation invalidates the previous generation's live status.
Project creation selects published, funded resource allocation IDs. The manager
prepares its private project and encrypted approval rooms before submitting the
frozen definition. This creates no capacity grant; the coordinator decides the
request and Hagency rechecks room membership before reporting readiness. Room
plans persist before creation and reconcile aliases/creation bindings after lost
replies. Agent forms derive every actor, room and authority binding server-side.
An access-token change reconciles the original Matrix request event before retry.

An owner can submit a top-up form through `palpo.inbox.submit` with
`kind: "token_top_up"`, `agentActionId`, a stable `requestId`,
`expectedAllocatedTokens` and a decimal-string `requestedAdditionalTokens`.
Rust loads the agent's project/resource bindings and current provider allocation;
the form cannot replace those bindings. Stale observations, changed allocations,
other users and requests already exceeding the engagement grant are refused.
An exact retry returns its original action even after the allocation changes.
The coordinator's normal Inbox decision queues a bounded native top-up command;
only Hagency's atomic reservation can make the additional tokens available.

The feature `rustWorkflowRequests: 1` identifies the new request schema.
Submission takes `{ "kind": "project" | "agent" | "token_top_up", "request": <typed request>, "definition": <immutable JSON> }`.
Approval takes `{ "id": <action>, "decision": "approve", "command": <typed approval> }`.
Rejection takes `id`, `decision: "reject"`, `expectedRevision`, `commandId` and
`reason`. See [HTTP integration tests](tests/workflows.rs) for complete payloads.
All command identities, request contents and current registration/delegation/
project revisions are checked against server state, not trusted from the caller.

The reviewed existing manifest can request its full service list; the session
grants only the implemented intersection. Unknown capabilities are refused.
The paired [Rinx adapter](https://github.com/hagency-org/Rinx/pull/65) follows
those grants and supports the Rust decision intent and read models. Project forms
send allocation IDs in `resourceIds`; agent forms additionally bind
`resourceAllocationId` while `agentDefinition.resourceId` identifies its parent
resource. A global catalog resource without a funded grant is never requestable.

## My Actions delivery

Configure `PALPO_ACTIONS_BOT_MXID`, `PALPO_ACTIONS_BOT_TOKEN_FILE` (a private
0600 regular file) and `PALPO_ACTIONS_PUBLIC_ORIGIN` to enable the Rust worker.
The token belongs only to a dedicated Matrix notification bot. It is never
stored in workflow JSON, returned to the mini-app, or included in cards.
`PALPO_ACTIONS_QUIET_START_UTC` and `PALPO_ACTIONS_QUIET_END_UTC` are minutes after
midnight (0–1439); equal values disable quiet hours. The worker runs every 30
seconds, checks canonical revisions and current visibility, verifies private
room membership/settings, and retries the same Matrix transaction after a lost
reply. Reminders occur after one hour, one day and two days while action remains
required. Seen/dismissed messages do not complete requests; snooze delays the
recipient's reminder. Notices contain an action route and identity binding,
never definitions, credentials or private project room details. A separately
retryable pinned board projects the latest authorized pending count.

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
before reactivation. Authenticated Hagency `coordinatorUpdates` now apply the
same monotonic checks for funded resources, authorized projects and decision
receipts. They cannot assign a coordinator or create an association. Native
publication ACKs compare the exact stored digest so a newer grant or ready
state queued during a retry is preserved.

Migration checks use a **copy** of an existing database. Both executables must
never own the same database concurrently. The Rust executable honors the Node
`${PALPO_ADMIN_DATABASE}.lock` convention, refuses existing locks and database
symlinks, and releases its lock on graceful shutdown. It never steals stale
locks; recovery requires confirming that the previous process has stopped.
New files use private permissions on Unix.

All legacy JSON (including credentials and unknown extension fields) and the
`fleet_delivery` table are preserved. Rust data lives in `rustWorkflows`.
Preservation is not a semantic migration: old Inbox records are **not converted**
into new coordinator requests. Existing delivery rows remain subject to the
same authenticated generation/lease protocol; their payloads are not rewritten
into coordinator commands. Neither legacy requests nor hostnames can establish
new role bindings. Explicit reconciliation is required before production cutover.

## Remaining cutover gates

1. Rust engagement association, designated-admin approval/profile export,
   coordinator delegation and real connection proof; independent registrations
   for multiple engagements, including those with the same Matrix hostname.
2. Rust equivalents for remaining fleet/admin routes and account operations.
   Relay/poll/ACK/updates, room preparation and Matrix action notifications are
   implemented; retirement and legacy reconciliation still need completion.
3. Hagency command authentication, hierarchy-aware transactional reservations,
   provisioning and receipts are implemented in the paired Hagency integration
   branch and still require combined acceptance. One coordinator decision must
   suffice; the Hagency portal must not add a second approval. Agent/top-up
   approvals must not silently enlarge their engagement or parent pool.
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
These are backend tests. The paired Rinx branch additionally exercises this
executable through its production OctoScript form and Makepad instrumentation,
using explicit Matrix and provider fixtures; it does not establish live Hagency
provisioning or chat.
