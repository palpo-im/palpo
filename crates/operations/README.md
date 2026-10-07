# Rust Operations service

`palpo-operations` implements the Rust workflow backend required by
[Rinx ADR 0011](https://github.com/hagency-org/Rinx/blob/main/docs/adr/0011-hagency-server-engagements.md).
It is a Rust executable and a mountable Salvo router. It never invokes Node,
serves JavaScript, or proxies business decisions to the old service. Rinx's
native host remains Rust and the mini-app presentation remains OctoScript.

The ADR workflow paths are implemented, including migration and scoped lifecycle
operations. Production cutover still requires the deployment and device gates
below. General homeserver diagnostics outside the ADR lifecycle are not claimed
as browser-admin parity. The historical Node code remains under `tests/fixtures/legacy_node/` for migration checks;
it must not write a database handed to Rust.

## Implemented

| Existing backend | Rust implementation | Boundary |
| --- | --- | --- |
| `lib/store.mjs` | `store.rs` | Existing SQLite `state` table, complete JSON preservation, process lock, atomic transactions |
| `lib/miniapp.mjs` | `api.rs`, `matrix.rs` | Borrowed Matrix login, scoped in-memory sessions, revalidation, disconnect |
| `lib/miniapp.mjs` project/agent reads | `views.rs` | Role-scoped pagination, distinct approval/execution state, lower-bound token observations with freshness |
| `lib/inbox.mjs` | `workflow.rs` | Typed project/agent/top-up requests, coordinator decisions, visibility, seen/snooze, durable receipts |
| `lib/action-notifications.mjs` | `notifications.rs` | Durable private My Actions rooms, idempotent delivery, reminders, quiet hours and pinned Inbox board |
| `lib/notification-preferences.mjs` | `preferences.rs` | Account-scoped revisions, replay-safe settings, daylight-saving quiet hours, overdue reminder coalescing |
| `lib/accounts.mjs` | `accounts.rs`, `accounts/` | Private signup receipts, legacy AES-GCM password sealing, bound Matrix verdicts, UIAA registration and original-device crash reconciliation |
| `lib/workflow.mjs` creation forms | `creation.rs`, `rooms.rs` | Funded catalog, owner room preparation, frozen project/agent submissions and lost-reply reconciliation |
| Association setup/export | `associations.rs`, `engagement_setup.rs`, `connections.rs` | Owner request, designated admin decision, recoverable appservice installation, scoped native profile export and authenticated connection probe |
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
Profile installation, association approval and connection verification have
separate persisted states and recovery paths.

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
- `palpo.fleets.list`, `palpo.fleets.install`, `palpo.fleets.export`, `palpo.fleets.connect`
- `palpo.agents.control`, `palpo.inbox.activate`
- `palpo.fleets.set_state`, `palpo.fleets.migrate`, `palpo.fleets.queue`
- `palpo.activity.list`
- `palpo.notifications.get`, `palpo.notifications.set`
- `palpo.actions.room.get`, `palpo.actions.room.ensure`
- `palpo.requests.open`, `palpo.accounts.list`, `palpo.accounts.open`

Notification preferences retain the legacy account-keyed representation for
rollback compatibility. Muting notices never resolves an action. Explicit room
setup can join the owner's invitation or replace an invalid private room with a
new revision. The read operation never creates or joins a room, and validates
membership, bot identity, privacy and the complete binding before returning it.

Set `PALPO_ACCOUNT_CONFIG` to the existing private (0600) account worker JSON
configuration to enable signup. Its bot/admin credentials, approver list,
password key and registration token stay in the Rust worker. The public
`GET /api/account-access`, `POST /api/account-requests` and
`POST /api/account-requests/status` retain the receipt-based protocol, with exact
origin/Host checks and bounded socket-peer rate limits. The mini-app can inspect
signup metadata and open a verified original Matrix approval event; it cannot
submit a verdict through the signup navigation service.

The worker preserves legacy request IDs, ciphertext/AAD, key/approver binding,
cursor, source events, history upgrades, registration device proof and terminal
decisions. Every start and pass verifies live bot/admin authority and private
room membership. Lost room creation and notice replies use stable aliases and
Matrix transaction IDs. Registration is durably marked before sending; after a
lost response, only the original random device proves ownership of an existing
ordinary account. Passwords are erased on terminal outcomes and registration
session tokens are never persisted. Approved registration retries do not expire
as if they were undecided requests.

Set `PALPO_ASSOCIATION_ADMIN` to the designated local Matrix administrator and
configure the transport/relay origins to enable owner association requests at
`POST /_palpo/miniapp/v1/association-request`. Hagency's authenticated native
`association` command verifies its owner account and persists the runtime/intent
before calling this route. Approval rechecks current admin authority and active
human owner/coordinator accounts. Installation failure retains the approval and
credentials; retry reconciles the existing appservice rather than reapproving.
Only the designated current admin and explicitly listed owner/coordinator may
export. The Rinx host writes secrets through its native save dialog.

Owner connection setup persists a stable room/probe before Matrix writes. A
current generation's exact relayed Matrix event, custody ACK and authenticated
Hagency receipt are required to verify it. `lastVerifiedAt` and heartbeat-derived
`connectivity` are separate; an old successful proof does not mean online.

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
resources, eligible managers and ready projects. Normal setup obtains these
through the approved association and authenticated runtime publications. Audited
legacy state can also be imported with an **offline operator command**, never an
app request:

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

`migration-inventory` emits a private-content-free inventory from a rollback-only
read transaction: record counts, source and delivery digests, and explicit stable
fleet/project/agent-allocation mappings. A legacy `provider.engagementId` remains
an **agent allocation**; the fleet ID identifies its server engagement. The
inventory contains no credential values or signup ciphertext.

After semantic reconciliation and the native accounting audit, the offline
`handoff-store <reviewed-handoff.json>` command accepts `{version:1,id,inventory,
nativeAuditDigest}`. It compares the complete current state and delivery digest
with the reviewed inventory and atomically records a receipt plus permanent
writer fences. The native audit digest is provenance, never a resource grant.
Changed source data or a reused operation ID with different content is refused.
An identical replay returns the original receipt without restoring old state.

The fence covers workflow JSON, custody delivery rows, ownership and migration
receipts. Old binaries lack its connection-local SQLite writer function, so
their writes fail even after the Rust service releases its process lock. Updated
Node startup refuses the database before serving traffic and removes its failed
startup lock. `new Store(path, {readOnly:true})` remains available for inspection.
Rollback uses a compatible Rust executable and the **current** database; never
restore a snapshot that predates post-cutover decisions. A current SQLite backup
retains the fences and original handoff receipts across restore/replay.

All legacy JSON (including credentials and unknown extension fields) and the
`fleet_delivery` table are preserved. Rust data lives in `rustWorkflows`.
Preservation is not a semantic migration: old Inbox records are **not converted**
into new coordinator requests. Existing delivery rows remain subject to the
same authenticated generation/lease protocol; their payloads are not rewritten
into coordinator commands. Neither legacy requests nor hostnames can establish
new role bindings. Explicit reconciliation is required before production cutover.

## Retirement and deployment acceptance

Whole-agent Matrix retirement is served at the authenticated fleet endpoint
`POST /api/fleet/v2/{fleet}/retire-agent`. Configure
`PALPO_RETIREMENT_ADMIN_TOKEN_FILE` with an operator-owned 0600 token file;
borrowed Rinx sessions are never retained for this worker. Each call rechecks
current administrator authority and exact request, namespace and appservice
ownership, rejects another live allocation of that identity, then verifies
deactivation, empty joined rooms and denied appservice authentication. Lost
replies retry the original identity. Paused/revoked fleets retain only this
restricted cleanup route under their current transport generation. Receipts do
not assert local process termination or settle token usage.

Four retirement HTTP scenarios cover lost replies/replay, changed server state,
partial native provisioning, scope/generation denial, other live allocations,
room removal and appservice denial. Rinx chat navigation is fenced as soon as
retirement starts. These are Matrix-fixture tests, not live deployment evidence.

The isolated live acceptance on 2026-10-05 ran Rust Palpo, the real Matrix
homeserver and native Hagency with two independent TLS engagements on the same
server. Actual Rinx forms approved projects/agents; both agents replied in DM and
project rooms from Android. Live transport rotation reattached the second agent
while the first stayed available. Top-up replay preserved one increase; retirement
verified runtime and Matrix cleanup while retaining unknown usage reservations.
The native execution peer was deterministic and supplied no paid-provider usage
measurement. Rinx's validation document records screenshots and run identifiers.

Before production cutover, audit the deployment's own copied database and native
ledger, complete each claimed platform's device checks, and deploy with only the
Rust writer. The migration tests establish preservation, adoption, rollback/replay
and fencing; they do not authorize replacing a real deployment. Rollback uses the
current database with a compatible Rust binary. Future job summaries and general
homeserver diagnostic administration are outside this first lifecycle release.

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
using explicit Matrix and provider fixtures. The separate isolated live acceptance
above additionally exercises native Hagency provisioning and Matrix chat.

Agent display names use the existing scoped `palpo.agents.control` service with
`operation: rename` and a bounded `displayName`. The provider must advertise
`coordinatorAgentProfileV1`. Receipt means the desired name was stored; only an
exact authenticated `lifecycle.matrixProfile` read-back marks it verified. Names
do not change allocation identity/history or block otherwise-ready chat. API
regressions cover role denial, capability negotiation, invalid labels, frozen
retries, verification failure and confirmation.

Approved project recovery uses `palpo.inbox.activate` with the action ID, a
frozen command ID and expected revision. It requires the project owner,
resource owner or current coordinator and `coordinatorProjectSetupV1`. The
runtime receives an immutable reference to the original approval; recovery
cannot create or change its resource grant. Current-generation observations
separately report pending, failed (with bounded room/authority reasons), or
ready. Transport ACK never establishes readiness. Matching retries preserve the
original decision and queue one command; superseded or conflicting observations
are refused. Native Rinx acceptance `0a97a9126f214590a91825d332085946` passed
22 checks using Rust Palpo and explicit Matrix/provider fixtures; the actual
Hagency process has a separate restart/join-repair test.

Legacy allocation adoption follows the storage handoff. Stop both runtime
writers, create complete SQLite backups (including committed WAL), and use the
native `hagency coordinator-migration --state-dir DIR inventory` / `adopt --file`
commands to authorize an explicit owner mapping against the audited native
ledger. Keep the resulting native receipt. With the verified engagement already
imported, obtain a fresh `palpo-operations migration-inventory`, then invoke
`palpo-operations adopt-legacy reviewed-adoption.json` with:

```json
{"version":1,"id":"migration_one","inventory":{},"nativeReceipt":{},"pendingAgents":{}}
```

Replace the empty inventory/receipt with their complete reviewed outputs.
`pendingAgents` maps original `fleetId:requestId` keys to explicit native resource
allocation IDs; these requests remain undecided and require current delegated
approval. Existing native allocations retain their IDs and original receipts.
Original Node records, project verdicts and unknown extensions remain unchanged.
No imported historical status grants readiness: a fresh authenticated observation
and the existing Matrix navigation checks are required. Exact replay survives
later decisions and complete-database restore without resetting them.

Run `tests/legacy_adoption.py --node NODE24 --binary target/debug/palpo-operations
--native-fixture FILE` with the isolated artifact emitted by Hagency's
`coordinator_migration` integration test through
`HAGENCY_MIGRATION_FIXTURE_OUTPUT`. This exercises the actual Node store and both
Rust CLIs, including restored-copy and post-adoption replay checks. Deployment
and device acceptance remain separate.

An installed outbound legacy fleet can upgrade in place through the authenticated
Hagency owner setup command, adding `--existing-fleet-id hf_…`. Its recorded owner
must initiate the request and the designated Matrix administrator reviews the
new runtime/delegation binding. Approval verifies the existing appservice and
retains its registration, transport generation/token, namespace and agent map.
It clears the old connection proof; import the exported profile into the same
Hagency installation and complete a new native probe before adopting capacity.
A second competing upgrade or another owner's request is refused. This supplies
the legacy profile/delegation transition described above.

Legacy project, contribution and agent requests remain visible as immutable
history under current account scope. Old raw request links resolve to an adopted
canonical action without duplicating it in lists. Continuing an eligible legacy
project retains its original request/decision history and binds current explicit
engagement allocations; neither historical approval nor server administration
creates a resource grant. Legacy contribution creation is replaced by owner-led
association setup, and direct agent identity registration is replaced by native
coordinator-approved provisioning. Their old capabilities are deliberately never
granted; they are not unfinished authority-bypass routes.

### Earlier project-role workflow proposal

[PR #507 at `8ccc7b0e`](https://github.com/palpo-im/palpo/tree/8ccc7b0e52f723e2c931946c9350a02a0824decd/web-admin)
implemented an earlier Node workflow with a designated server administrator for
project approval and assigned project administrators for agent approval.
ADR 0011 supersedes that authority model. The final integration preserves the
proposal in Git history and keeps the current Rust implementation; it does not
install the old Node services alongside it.

| Earlier proposal | Current Rust path |
| --- | --- |
| Project and agent decisions, scoped commands and execution receipts | `workflow.rs`, `outbound.rs`, `updates.rs` and `palpo-hagency-contract`; authority comes from the current engagement delegation |
| Agent top-ups and removal | `workflow.rs`, `lifecycle.rs` and `retirement.rs`; execution and verified cleanup remain separate from a human decision |
| Reminders, quiet hours and private My Actions rooms | `preferences.rs`, `notifications.rs` and `notifications/room_api.rs` |
| Opening an agent conversation or a signup approval | `navigation.rs` and `accounts.rs`; navigation rechecks the current account and verified room binding |
| Old approvals and allocations | Explicit handoff, adoption and continuation described above; an old verdict never grants new authority |

The old branch's 148 Node service tests passed in GitHub run
[`37266711118`](https://github.com/palpo-im/palpo/actions/runs/37266711118), but its
browser suite timed out waiting for `#request-role-hint`. That historical failure
is not a passing acceptance result for either backend. Current validation uses
the Rust tests and native-client evidence described here; it does not claim
behavioral parity with the superseded approval policy.
