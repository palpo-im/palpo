# Palpo HAFleet web administration

This application lets a **Palpo server administrator** authorize a HAFleet and install its isolated App Service. The HAFleet owner pairs their service and verifies an actual event round trip; project owners create/register projects and encrypted private approval rooms, request roles, and follow manual HAFleet approval through verified Matrix agent admission. Administrators can inspect, update and retire managed identities. It runs as a separate Node service beside Palpo and uses supported Palpo APIs.

This implements the main onboarding/request path of `REQ-PALPO-HAFLEET-ONBOARDING`, with remaining gaps tracked below. Installation alone stays **pending connection**. **Ready** requires the exact App Service event receipt and verified reception memberships. An approved request becomes usable only after HAFleet reports complete fulfillment and the actual agent is observed in the target project. Local task execution and runtime approval remain HAFleet responsibilities.

New fleets use a durable **outbound connection**: HAFleet polls the public Palpo
transport, and Palpo receives Matrix transactions on its internal Docker network.
Capabilities and request status are published by HAFleet. Established offline
fleets can queue requests against their last published resources; allocation and
actual Matrix admission remain separate checks. Existing callback fleets require
an explicit administrator migration. See [the outbound protocol and deployment
guide](deploy/outbound-v2.md) for authentication, leases, rotation and capacity.

## Run

Node 24 or newer is required for its built-in SQLite module. There are no production package dependencies and no asset build step.

```sh
cd web-admin
PALPO_URL=http://127.0.0.1:8008 \
PALPO_SERVER_NAME=example.org \
PALPO_TRANSPORT_ORIGIN=https://admin.example.org \
PALPO_RELAY_ORIGIN=http://palpo-web-admin:8090 \
PUBLIC_ORIGIN=https://admin.example.org \
npm start
```

Optional administrator-reviewed account signup is documented in [account approvals](deploy/account-approval.md). It adds a public request form and a private Robrix approval room; Agent allocation still requires HAFleet approval.

Open the configured `PUBLIC_ORIGIN` through its reverse proxy and sign in with an existing **local Matrix account and password**. Server administrators get management controls. Ordinary HAFleet owners and project members get the member workflow. Every browser request revalidates its actual Matrix identity, and admin routes additionally require current Palpo administrator authority. An App Service authentication failure retains the valid browser session and failed operation; it is not mistaken for an expired user login.

`PALPO_URL` is a fixed server-controlled upstream origin; browser callers cannot change it. `PALPO_SERVER_NAME` is the actual Matrix identity domain, not a display label. The database is bound to that origin and server name and refuses reuse with another server.

For explicit legacy callback mode, `PALPO_CALLBACK_ORIGINS` is a comma-separated list of origins that the **server operator has approved as event destinations**. It defaults to an empty list. No caller can submit arbitrary namespaces or broaden a callback beyond that policy. Allowed origins are trusted infrastructure: the app does not independently validate DNS, pin addresses, or protect Palpo from DNS rebinding at an allowed destination. Palpo's own outbound network policy must also allow and constrain these destinations.

For a public installation, set an HTTPS `PUBLIC_ORIGIN`, put this process behind an HTTPS reverse proxy, and preserve the public `Host` header. The process listens on `127.0.0.1` by default; override `LISTEN_HOST` only as part of the server's deployment configuration. Cross-origin browser mutations and unexpected `Host` values are refused.

Other settings:

| Variable | Default |
|---|---|
| `PORT` | `8090` |
| `LISTEN_HOST` | `127.0.0.1` |
| `PALPO_ADMIN_DATABASE` | `web-admin/data/admin.sqlite` |
| `PUBLIC_ORIGIN` | `http://127.0.0.1:8090` |

The SQLite database contains App Service credentials, operation plans and audit records. It is created with mode `0600` under a private directory. Keep it out of source control and back it up as secret server data. Administrator access tokens stay only in memory. Browser sessions expire after 30 minutes and use HttpOnly, SameSite=Strict cookies (Secure on HTTPS) and CSRF tokens. Passwords are forwarded only to the fixed Palpo login API and cleared from the form.

Run one process per database. A process lock prevents competing installations. Graceful shutdown removes it. After an ungraceful exit, verify the PID in `<database>.lock` is no longer running before removing **only that stale lock** and restarting. This release does not automatically recover stale process locks or revoke orphaned Matrix login devices after a process crash.

## Implemented workflow

1. The administrator authorizes a fleet by selecting an active local owner, a display name, outbound transport (or an explicitly allowed legacy callback) and a stable request ID.
2. The service persists the operation, random fleet ID, generated credentials, representative and exclusive namespace before contacting Palpo. Retrying the same request reuses these values. Changed content with the same request ID returns `409`.
3. It installs the registration through the actual Palpo admin API and reads it back to verify ID, callback, both tokens, sender, namespace and event options. Changed upstream credentials are reported as drift; they are never silently replaced.
4. It provisions the representative through Palpo's authenticated App Service `whoami?user_id=…` path, which creates a real user/profile/device on this Palpo version. It then verifies the exact returned MXID and the administrator-visible `appservice_id`. Registration alone is not reported as identity provisioning.
5. The owner retrieves **only their own fleet's** version 1 credentials through the pairing API or explicit configuration download. Retries return the same version and tokens. No list/detail browser response contains raw service tokens. Installation still remains pending connection until the connection workflow succeeds.
6. An administrator can create a managed agent identity with a stable agent ID, public role and independently approved request reference. This reference is **administrator attestation** in this initial slice; it is not a verified HAFleet engagement decision. Creation does not consume HAFleet tokens, admit an agent to a project, or start a runtime.
7. Agent reads observe actual Matrix profile, ownership and joined rooms, with an observation timestamp. Read failures and local runtime health remain unknown. Updating the display name cannot alter the MXID, owner, role, or local runtime configuration.
8. Retiring an identity invokes Palpo's full deactivation API with history erasure disabled, then verifies deactivation, removal from joined rooms and rejection of App Service authentication for that identity. Partial failure remains `retiring` and can be retried. Local task stop remains **unconfirmed**.

### Owner and project workflow

For outbound fleets, owner verification establishes one exact Matrix proof per
credential generation. Heartbeats maintain online status for 90 seconds without
a browser. Repeated Verify actions reuse that generation's event, including after
a lost publication response. Only credential rotation starts a new challenge.
Request usability additionally requires a current-generation status observed and
received in the last 90 seconds and actual target membership; heartbeats do not renew stale
request statuses. Offline verified fleets keep Send available for durable queuing.

For legacy callback fleets, the agent request form checks connection expiry as well as
its published roles and project readiness. Expired verification disables Send
agent request and explains the recovery beside the form. While the member page
is visible, a signed-in fleet owner automatically renews previously verified
connections when they expire or have less than one minute left. This runs on
sign-in, catalog polling and return to the page, using the same owner-authenticated
connection endpoint and exact pushed-event/membership proof. A valid proof stays
usable during renewal; a failed proof or failed status readback disables Send,
shows the error and waits at least one minute before an automatic retry. Owners
can also use Verify connection immediately. Initial pairing/reception creation
remains manual, and paused/revoked fleets are never automatically resumed.

Signing in refreshes the browser session, which is separate from the fleet's
connection proof. Automatic renewal needs the fleet owner's current session;
another project owner/member cannot renew on their behalf. Hidden pages and
signed-out sessions do not renew connections. Other members see which owner must
reconnect. The 30-minute proof lifetime and browser session lifetime are unchanged.
Reconnection performs the existing real event check, preserves the reception
room and request fields, and does not submit or approve resources. Submission
results and failures appear beside the form; failed submissions retain their
request ID for retry. Renewal never retries an agent submission automatically.

1. A HAFleet owner signs in with their own Matrix account and downloads the assigned configuration from **My HAFleet access**. Import it into the isolated HAFleet runtime; it contains only that fleet's credentials, never a Palpo administrator token.
2. **Verify connection & create reception** reads HAFleet capabilities (published snapshots for outbound fleets), creates or recovers the exact private plaintext reception room, joins its owner, and sends `com.hafleet.connection.probe.v1` through Matrix. HAFleet must prove receipt of that exact event through the actual App Service delivery. For outbound transport, Palpo requires the corresponding persisted transaction ACK and current-generation probe receipt. Merely reading the event back does not count. A failed check remains pending and retries the same event. Legacy callback evidence is valid for 30 minutes; an expired legacy check cannot authorize new requests. Outbound evidence follows the generation and heartbeat rules above.
3. A project owner signs in with their own Matrix account. **Create project and approval room** either creates an invite-only plaintext project or verifies a selected existing room. The owner must have power level 100. It installs a fleet-scoped project state binding from the owner's Matrix session, invites/joins the representative, and creates a separate invite-only Megolm approval room containing exactly the owner and the actual HAFleet approval bot. A bot that has not joined keeps the project pending.
4. A project owner or authorized member selects the registered target, published role and quota. Membership and invite authority are rechecked. The browser never supplies an arbitrary alternate target room or owner binding. The request's actual Matrix event is sent as the requester in the reception room; the private approval-room ID travels only over the authenticated per-fleet HTTP channel.
5. HAFleet independently verifies the source event, target state binding, requester/owner authority and private approval room. Its operator makes the resource decision in the HAFleet console. Refreshing the project request status shows pending/preparation/failure and the actual allocated agent. **Open project and use agent** appears only after complete HAFleet fulfillment and verified target membership. Unknown or failed observations never become usable.

Pause/resume verify the actual App Service disabled flag. **Revoke service** is final in this release and disables that App Service credential; its API response includes `revocationScope: "appservice_credentials_only"`. It does not claim to revoke separately issued Matrix user sessions or remove every identity's room membership. Use each identity's retirement flow for that scope. Fleet-wide retirement, HAFleet notification and local stop acknowledgement remain required follow-up work.

## API

The browser and programmatic administrator use the same `/api` endpoints. Admin mutations require a valid session cookie, exact `Origin: <PUBLIC_ORIGIN>` and `X-CSRF-Token` returned by sign-in/session. Tokens from upstream Palpo responses are never forwarded by these endpoints.

| Method | Endpoint | Result |
|---|---|---|
| `POST` | `/api/login` | `{username, password}` → session cookie and CSRF token |
| `GET` | `/api/session` | Current administrator, CSRF token and public server policy |
| `POST` | `/api/logout` | Revoke this Matrix session and clear the browser session |
| `GET`, `POST` | `/api/fleets` | Redacted list; authorize/install `{requestId, name, ownerMxid, transportMode?:"outbound"}`; explicit `"callback"` also requires `callbackUrl` |
| `GET` | `/api/fleets/:fleetId` | Redacted operation and registration state |
| `GET`, `POST` | `/api/fleets/:fleetId/outbound` | Inspect queue usage/limits; migrate or rotate with `{requestId, rotate?:true}` |
| `POST` | `/api/fleets/:fleetId/install` | Retry the saved installation |
| `POST` | `/api/fleets/:fleetId/pause`, `/resume`, `/revoke` | Verify service credential state |
| `GET`, `POST` | `/api/fleets/:fleetId/agents` | Observed identities; create `{agentId, displayName, role, approvedRequestId}` |
| `PATCH` | `/api/fleets/:fleetId/agents/:agentId` | `{displayName}` only |
| `POST` | `/api/fleets/:fleetId/agents/:agentId/retire` | Deactivate and verify Matrix retirement |
| `GET` | `/api/audit` | Last 200 operation records, without credentials |

Any signed-in local Matrix member can use the project routes; owner-only operations check the exact recorded full MXID:

| Method | Endpoint | Result |
|---|---|---|
| `GET` | `/api/catalog` | Installed active fleets and their last verified public offers |
| `GET` | `/api/my/fleets` | Only fleets owned by the current Matrix user |
| `POST` | `/api/my/fleets/:fleetId/pair` | Explicit resumable owner configuration download |
| `POST` | `/api/my/fleets/:fleetId/connect` | Capability identity check, reception setup and exact pushed-event receipt verification |
| `GET`, `POST` | `/api/projects` | Authorized projects; create/register `{fleetId, requestId, name, roomId?}` and encrypted private owner room |
| `GET`, `POST` | `/api/requests` | Own/requested project status; submit `{projectId, requestId, role, requestedTokens, ratePerDay}` |

### Legacy callback protocol, version 1

All four endpoints are on the configured App Service callback listener, authenticated with `Authorization: Bearer <this fleet's hs_token>`. The Palpo admin app never receives the HAFleet operator API token.

- `GET /api/fleet/v1/capabilities` returns `{v:1,fleetId,serverName,representativeMxid,approvalBotMxid,offers:[{role}]}`.
- `POST /api/fleet/v1/probe` accepts `{fleetId,sourceRoomId,sourceEventId,challenge}`. Success must return the same fields plus `received:true` and prove a prior App Service push receipt. HTTP event readback alone is insufficient.
- `POST /api/fleet/v1/requests` accepts `{v:1,fleetId,requestId,requesterMxid,sourceRoomId,sourceEventId,targetProjectId,targetRoomId,ownerMxid,ownerDmRoomId,role,requestedTokens,ratePerDay,authVersion:1}`. The Matrix event type is `com.hafleet.engagement.request.v1` and contains the same fields **except sourceEventId and ownerDmRoomId**. It is sent with the actual requester's Matrix session. Private owner-room binding is only in the authenticated HTTP channel.
- `GET /api/fleet/v1/requests/:requestId` returns the durable binding and public decision/fulfillment state. The application compares fleet/request/source event/source room/target project/target room before accepting status. Arbitrary extra fields are not forwarded to browsers.

The project owner's Matrix session writes state type `com.hafleet.admin.binding.v1`, state key equal to the fleet ID, with `{v:1,fleetId,purpose:'project',projectId,ownerMxid,authVersion:1}`. HAFleet verifies that state plus current room owner, requester membership/invite authority, representative membership/invite authority, and the exact private encrypted approval-room participants. Room display names and source reception membership alone cannot authorize another target.

Programmatic owner pairing is separate from administrator sessions; the owner browser also has an explicit configuration download action:

```http
POST /api/pair/hf_<server-generated-id>
Authorization: Bearer <owner's Matrix access token>
```

The service asks the fixed Palpo server who owns that token and compares the exact full MXID with the fleet's recorded owner. A different owner gets `404`; a browser administrator session cannot substitute for the owner token. The first successful response contains `{fleetId, serverName, credentialVersion, registration}` including that fleet's `as_token` and `hs_token`. Store it in the HAFleet credential store, not browser local storage or logs. The equivalent authenticated owner download is `POST /api/my/fleets/:fleetId/pair`; these explicit delivery operations are the only credential-returning endpoints.

Delivery is durably recorded before responding. Repeating it with the same verified owner returns the same credential version and values, so a lost response can be recovered without duplicate registration or credential replacement. Effective App Service identity is reverified before each delivery. The separate outbound transport secret supports explicit administrator rotation with a new generation. Matrix App Service token rotation is not implemented; ordinary pairing retries recover the current values.

## Palpo endpoint verification

Reviewed against upstream commit `3e4fbd332fe3845e99d91884d812493189f91860`. No behavior is assumed from Synapse compatibility alone:

| Palpo source | Behavior used |
|---|---|
| `crates/server/src/routing/admin.rs` | Access-token authentication plus `require_admin` for `/_palpo/admin` |
| `crates/server/src/routing/admin/appservice.rs` | v1 list, full detail, register, disable and enable |
| `crates/core/src/appservice.rs` | Empty namespace collections are omitted on serialization |
| `crates/server/src/hoops/auth.rs` | Enabled App Service token lookup; namespace-scoped virtual user creation on authenticated `user_id`; account usability enforcement |
| `crates/server/src/routing/client/account.rs` | Actual authenticated identity returned by `account/whoami` |
| `crates/server/src/routing/admin/user_admin.rs` | v2 user profile/ownership, v1 joined rooms and full deactivation |

Existing namespaces are accepted only where an anchored literal prefix proves them disjoint from the newly allocated namespace. Broad, ambiguous or unsupported patterns fail closed and require administrator review. The application manages its own fleet registry; it does not import arbitrary pre-existing App Services or display their secret-bearing detail responses.

## Requirements coverage

The source requirement is `knowledge/requirements/req-palpo-hafleet-onboarding.md` in the HAFleet repository, authored 2026-09-06. It is a Proposed requirement, not an existing accepted Palpo contract. The suffixes below refer to `REQ-PALPO-HAFLEET-ONBOARDING-*`.

| Requirement | Initial implementation / remaining work |
|---|---|
| API | Administrator, fleet-owner and project-member browser flows use the same APIs. |
| GRANT | Explicit administrator grant bound to a verified local human MXID; invitation/preapproval policy remains. |
| ISOLATION | Random stable fleet/service/representative/namespace; foreign-owner pairing rejected. Scoped owner CRUD remains. |
| AUTHORITY | Admin tokens server-only; redacted public projections; no generic admin proxy. |
| INSTALL | Controlled registration and read-back verification plus actual representative provisioning. Dynamic App Service authentication uses the current enabled database registration. |
| CONNECTION | Exact delivered event receipt and owner/representative membership gate readiness; outbound proof is generation-bound with heartbeat liveness, while legacy evidence expires. |
| NETWORK | Fixed upstream and operator callback-origin allowlist; generated namespace and conservative collision rejection. DNS/egress hardening is a deployment dependency. |
| CREDENTIALS | Resumable owner-bound same-version delivery and service credential revocation. Outbound machine-secret generation rotation is supported; Matrix token rotation remains. |
| IDEMPOTENCY | Durable fleet/project/room/request plans, deterministic room binding recovery, stable Matrix transaction IDs and content conflicts. |
| RECEPTION | Idempotent private reception creation with owner/representative membership verification and Matrix link. |
| READINESS | Installed versus pending connection versus verified ready, plus paused/revoked and partial failure. Readiness and request usability require independent evidence. |
| OFFERS | Outbound catalog reads published snapshots; legacy refresh reads current scoped roles with at most three callbacks at once; failures preserve prior offers and remain distinct from a verified empty list. Submission rechecks roles. Broader availability descriptions/public configuration and background refresh remain. |
| TARGET | Owner-created or owner-registered target with exact fleet-scoped Matrix state binding, current owner power and requester invite checks. |
| REQUEST | Durable source event, real requester, verified target, owner/version, role and quotas; fixed body checked independently by HAFleet. Manual admin identity creation remains separately labeled attestation. |
| VERDICT | Submitted requests become manually pending in HAFleet; the HAFleet console owns the actual resource decision. |
| FULFILLMENT | Scoped HAFleet status is source/target-bound; complete fulfillment and actual Matrix membership are required before usable and managed-identity linkage. |
| DELIVERY | Owner/member web request status tracks the original context; task messages/results stay in the target project through HAFleet. Private approval-room IDs never enter reception events. |
| AGENT-CREATE | Actual owned Matrix creation for manual admin grants; verified HAFleet fulfillment automatically links its actual admitted identity. |
| AGENT-READ | Exact identity, fleet, role, project/engagement link and observed membership; local runtime health remains unknown. |
| AGENT-UPDATE | Display-name update with immutable identity/ownership. Authorized project membership changes remain. |
| AGENT-RETIRE | Full Matrix deactivation and authentication/membership verification; history retained. HAFleet final-allocation revoke sends a scoped outbound retirement with local stop acknowledgement. Manual Palpo-only retirement cannot confirm local stop. |
| REVOKE | Service credential disable and no new managed identity creation; independent sessions/identity memberships require individual retirement. Fleet-wide cleanup and runtime stop acknowledgement remain. |
| AUDIT | Durable actor/object/time/result records for implemented workflows. Comprehensive denied-operation and distributed lifecycle auditing remains. |

Additional limitations: password login only (no SSO/MAS UI), local owners only, a single server and process per database, no imported legacy fleets, no background reconciliation or automatic retries, no pagination of managed fleet/agent lists, and no encryption at rest beyond filesystem permissions. Matrix token rotation, fleet-wide retirement and local stop acknowledgement for manual Palpo-only retirement remain explicit gaps. Live deployment/acceptance evidence is recorded separately by the deployment operator; fixture passes alone do not establish it.

## HAFleet final-allocation retirement

HAFleet initiates `POST /api/fleet/v2/:fleetId/retire-agent` using its current
outbound credential and generation after revoking an Agent's last allocation.
Palpo verifies the known request's exact Agent MXID and actual App Service
ownership, deactivates the account with `erase: false`, checks zero joined rooms
and refused App Service authentication, and retires every management record for
that MXID. The shared registration, representative and other Agents remain.
Late status publication cannot revive the retired request. Another active or
pending request using the same Agent prevents whole-account retirement.

The server needs an administrator token from its protected account-approval
configuration, or an explicit `PALPO_AGENT_ADMIN_TOKEN_FILE`. The token stays on
the server; HAFleet receives only scoped retirement results. Missing configuration
returns503. Incomplete removal is reported for explicit retry. The acknowledged
local stop is supplied by HAFleet, not inferred from Matrix membership. Existing
requests and management IDs remain as history, and replays preserve their
original revocation time.

## Validation

```sh
npm run check
npm test
```

The deterministic tests exercise actual HTTP authentication/CSRF and service calls against controlled Palpo fixtures. They cover token redaction, owner isolation, callback policy, namespace conflicts, content-bound retry, registration drift, real-identity verification, profile immutability, partial retirement, pause/revoke and persistence. They never connect to a live deployment or a model.

For a real Chromium browser over the same controlled Palpo fixture:

```sh
# Install playwright-core as a local test dependency, or provide its module path.
PLAYWRIGHT_MODULE=/absolute/path/to/playwright-core/index.mjs \
CHROME_EXECUTABLE=/absolute/path/to/chrome \
npm run test:browser
```

The first browser test signs in as administrator, then a distinct fleet owner and project owner; it covers installation, actual identity CRUD, connection/reception, project and private approval room creation, manual-pending request and fixture-approved admission tracking. It rejects credential leakage in list/status responses and saves screenshots plus `test-results/browser-summary.json`. It starts its own local server and browser; it does not use an existing operator browser or any live Palpo server.

The separate request-readiness browser regression covers owner sign-in renewal,
nonowner isolation, early renewal with a still-valid proof, coalescing, mismatched
receipt and catalog-readback failures, retry backoff, hidden/visible transitions,
a stale submission rejected without automatic replay, preserved request fields,
and exactly one manually pending request after explicit submission. Its controlled
fixture evidence is `test-results/request-readiness-summary.json`.

The outbound suite adds real HTTP and file-backed SQLite restart coverage,
immutable sequence/binding checks, lease expiry, migration/rotation, queue
backpressure, stale status isolation and malformed update rejection. Its
Chromium flow covers outbound authorization, owner download, exact relay proof,
offline resource selection and durable request submission, including a guard
against reverse HAFleet callbacks.

The account suite covers signup, the private approval receipt, login, project
creation and an Agent request that stays pending HAFleet approval. The Matrix
administrator verdict is supplied by the fixture; it is not a native Robrix
interaction. See [account approvals](deploy/account-approval.md).

GitHub Actions runs syntax checks, Node tests and all four Chromium workflows.
Server changes are covered by the existing Rust checks, including the opt-in
PostgreSQL regressions for dynamic App Service authentication and atomic URL
updates. Use an empty, dedicated `PALPO_TEST_DATABASE_URL` for those tests; the
fixture refuses an existing populated database. Live Matrix, Robrix, HAFleet
runtime and model acceptance require a separate deployment test.
