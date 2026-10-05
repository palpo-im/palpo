# Rinx Palpo mini-app adapter

This adds a native frontend to the existing web-admin backend. It reuses its
Palpo client, Workflow, Service, audit and SQLite store. Existing browser login,
cookie and CSRF routes remain available. No second user password is required.

## Routes and authority

Expose these POST routes on the **same HTTPS origin as Rinx's authenticated
Matrix homeserver**, forwarding to this web-admin process:

- `/_palpo/miniapp/v1/session`: current Matrix bearer, exact app ID
  `im.palpo.operations`, verified bundle digest and reviewed service names.
- `/_palpo/miniapp/v1/call`: short-lived app bearer, exact `service` and `args`.
- `/_palpo/miniapp/v1/disconnect`: discard only the app session.

Configure the proxy's upstream `Host` to match `PUBLIC_ORIGIN` exactly, including
its port. Forward Authorization unchanged; do not log its value or request/response
bodies. Do not rewrite an arbitrary client-supplied upstream URL. `PALPO_URL` and
`PALPO_SERVER_NAME` must identify the same Matrix server served by that public
origin. The adapter rejects browser Origin/Cookie context, redirects and arbitrary
service arguments. Browser /api routes keep their original CSRF enforcement.

The app bearer is random, stored by hash, expires after 15 minutes and is not
persisted. Its server-side record borrows the host's Matrix token; disconnect
never calls Matrix logout. Every call checks current Matrix identity. Admin and
owner operations reuse live role/ownership checks; mutations recheck after the
shared serial queue. The bundle's grant can narrow authority, never create it.
These endpoints trust an authenticated native client to describe its reviewed
bundle; the server does not attest the caller's Rinx binary or publisher key.

## Project approval migration

Set `PALPO_PROJECT_APPROVER=@admin:example.org` to designate **one** project
approval administrator. This identity must also retain live Matrix server-admin
authority. Other server admins and project managers cannot inspect other users'
Inbox requests or decide their projects. The role is checked for every decision,
including after waiting in the mutation queue. `canApproveProjects` in the app
session drives the dedicated Project approvals screen; `isAdmin` alone does not.

For migration, when this variable is absent, exactly one `approvers` entry in
`PALPO_ACTION_CONFIG` is used. Zero or multiple entries leave project approval
unconfigured: new requests and all approval decisions fail closed. Set the
explicit variable before enabling a multi-recipient legacy configuration.

Set `PALPO_PROJECT_APPROVAL_REQUIRED=1` to require approval for every **new**
project, including requests through the existing browser/API. Existing projects
and their original owners remain in place. Without this option, legacy direct
project creation remains enabled; do not describe that deployment as enforcing
mandatory project approval. New agent requests through either frontend require
an accepted finite allocation. An exact retry of an existing legacy operation
can continue; it never creates an implicit budget or changes the original verdict.

Resources originate in Hagency. Rinx project managers select resources already
published by a connected Hagency and request projects; they cannot contribute
resources or register a fleet. The mini-app backend rejects both contribution
submission and direct fleet registration, including from old bundles. Existing
contribution records remain readable and existing connections/projects are
preserved. The Rinx admin page monitors connections and retains authorized
maintenance of existing registrations; it does not create a contribution.
The established operator setup/import path remains available. Automatic
Hagency-originated pairing is a separate backend feature, not added here.

### Project command transport (development)

`ProjectCommands` adds a closed version 1 protocol over the authenticated outbound
work lane. Enqueue requires the same `Store.atomic` transaction as its decision.
Transport ACKs cannot change a project to allocated. Hagency publishes immutable
business receipts, which are checked against the original command, generation,
digest, project and result before committing with the update sequence.

The fixed machine `POST /api/fleet/v2/:id/authorize-command` route rechecks current
Matrix identity and designated/project-admin scope, returning a ten-second lease.
The server authority token comes from the existing private action/account config.
Lookup outages return retryable 503; a locked or demoted decision maker cannot
use the original queued command as continuing authority. Hagency rechecks lease
expiry after its writer lock. This bounds the distributed authorization window;
it does not guarantee instantaneous revocation of an already issued lease.

The shared wire corpus is `test/fixtures/project-commands.json`, identical to
Hagency `native/fixtures/project-commands.json`. Tests cover atomic enqueue,
receipt rollback/replay, assigned-admin scope, pending reassignment, HTTP
credentials/origin, outages and cross-language command digests.

Project requests now specify finite `allocations` (contribution/resource IDs,
tokens, maximum agents, aggregate daily rate and duration). Only fresh, active
contributions in the current registration and transport generation are offered.
Pending commands and accepted grants reduce the budget offered to other projects.

Submission prepares the actual owner-bound room using the owner's current
Matrix token and stable operation ID. This creates a proposal, not an allocated
project. A lost Matrix response resumes the same room. The designated admin
reviews that room and assigns active local Matrix users as project administrators,
with an explicit self-approval policy. Current room privacy, binding and owner
powers are rechecked before approval. The decision, reservation commands, audit
and notification intents commit in one transaction. `awaiting_reservation`
becomes `done` only after every exact business receipt is applied; refusals remain
unallocated. Partial reservations stay held, with no speculative refund.

### Assigned project-admin decisions and token increases

An agent requested against an accepted finite grant creates an `agent` Inbox
action after its original Matrix source event is acknowledged. Only explicitly
assigned project administrators can decide it, subject to the grant's self-approval
policy. A server-admin flag alone grants neither access nor decision authority.
The manager can read the result; no second human approval is sent to Hagency's
legacy console queue. Existing historical requests retain their original path.

Decision, exact source-bound command, audit and notices commit atomically.
Current identity, grant revision, room binding, contribution and remaining
capacity are checked before admission. A refused or replayed command cannot
invent a second allocation. An applied decision means admission/provisioning;
agent readiness still requires actual fulfillment and verified Matrix membership.
Because status and receipt publication are independently paginated, a status
whose decision receipt has not arrived is deferred without rejecting the batch.

Owners request additional tokens for the same approved agent through a `top_up`
Inbox action. The assigned admin may approve a smaller positive increase or
reject it. Rejection does not enqueue remote work. Approval references the same
engagement and consumes the remaining project budget once; no new agent request
or Matrix source event is created. Request projections distinguish confirmed
tokens from increases still awaiting a business receipt. Capacity is held
conservatively until a defined, verified lifecycle result releases it.

The full workflow remains **development-only**. Hagency does not advertise the
new capability until the remaining lifecycle and cross-service acceptance checks
are complete. Local fixtures explicitly supply the capability
and business receipts; they do not prove a live agent lifecycle. Old projects
need an explicit budget migration, and partial-refusal recovery remains work.
There is no implicit unlimited grant or legacy owner transfer.

### Agent removal and usage

An owner or currently assigned project administrator can submit `agent_removal`
through the existing scoped Inbox service. This is an immediate revocation intent,
with a reason and stable request ID; it does not introduce another human approval.
Cleanup is permitted after grant expiry/revocation, with the same fleet, project,
owner, registration and current administrator checks. Queue, audit, action and
notice writes are atomic. Top-ups stop while removal is outstanding.

An applied `revoke_agent` receipt means removal has started. Completion requires
both runtime custody/local cleanup evidence and Palpo's verified Matrix account
deactivation/App Service denial. The runtime uses the recorded agent identity.
A provider's `matrixRetired` claim alone is insufficient. Never-provisioned agents
need explicit `not_required` local cleanup and no Matrix identity. Old status
cannot revive completed removal, and an older action link opens its latest retry.

Definite cleanup failure permits a fresh command; an uncertain effect requires
operator inspection. The physical cleanup attempt fence prevents an old failure
page from offering another retry after a new command. Verified removal releases
concurrency and daily-rate allowance. Allocated tokens remain lifetime debits;
the observed usage lower bound cannot justify a refund. Rinx shows unknown usage
and stale observations explicitly, and keeps removal progress in the Inbox.

## My Actions notifications

Inbox works without a notification bot. To enable Matrix delivery, set
`PALPO_ACTION_CONFIG` to a private JSON file (0600), outside the checkout:

```json
{
  "homeserverOrigin": "https://matrix.example.org",
  "botMxid": "@palpo-actions:example.org",
  "botToken": "<dedicated bot Matrix token>",
  "adminToken": "<server-side administrator Matrix token>",
  "approvers": ["@admin:example.org"]
}
```

The bot creates a private non-federated My Actions room per recipient and invites
them. Each delivery verifies the binding, membership and restrictive state/power
settings; an unsafe room stops delivery and leaves the Inbox intact. Room notices
contain only generic text, an opaque action reference and a same-origin link.
They are plaintext minimal notices, not encrypted configuration delivery.
Only the designated project administrator receives project-review notices.
Current server-admin authority is rechecked before delivery. An old queued
notice for another administrator is cancelled rather than treated as a grant.
Agent and top-up notices instead target the owner/requester and explicitly
assigned project administrators. Delivery rechecks the current grant and active
recipient; removed assignments and locked/deactivated accounts receive no new
notice or reminder.

Workflow, audit and notification intent are committed together in SQLite. A
lost send receipt repeats the same Matrix transaction ID. Pending actions get
up to three reminders at one hour, one day and two days after that revision;
failures retry with capped backoff. Reading does not complete the action or stop
reminders. Snooze postpones delivery. A changed revision cancels obsolete notices.
The room's older messages remain history and their links open the latest result.
Quiet hours and the Glance-style room board are not implemented in this revision.

## Signup navigation

The Signups page uses `palpo.accounts.list` for its existing administrator list.
Its Open signup request action requires the separate `palpo.accounts.open` grant
and the host feature `palpo-account-navigation-v1`. Only a configured, currently
active account approver receives an authorized destination. Project approval
authority does not grant account approval authority.

The service accepts only a request ID. It verifies the private approval room,
the approver's invitation/membership, the original bot message, request digest,
tool and authorized approvers, then rechecks current administrator authority and
that the source has not been replaced. Rinx binds the result to its current
account and navigates to that exact Matrix event; it preserves the event while
the user accepts an invitation. Opening neither joins automatically nor decides
the request. Approve/reject still uses the existing trusted Matrix controls and
account worker's original-source/verdict checks.

The native fixture exercises the real page and adapter, recording the authorized
handoff without an SDK login. It proves no account or verdict is created by
opening, but does not prove timeline rendering or a real Matrix signup decision.

## Validation and release limits

Run Node 24+: `cd web-admin && node --test test/*.test.mjs`.
The native fixture is `test/miniapp-native-fixture.mjs`; Rinx's
`tools/wechat-ux/live/native_palpo.py` starts it, drives the production Splash
frontend through Makepad, and cleans up. HTTP, sessions and SQLite workflows are
real; Matrix/Hagency observations are explicit local test fixtures.

This is an implementation milestone, not completed ADR acceptance. Remaining:
public deployment and native file saving/runtime import, richer My Actions room UI,
actual Hagency command/receipt and runtime/Matrix cleanup integration,
signup decision parity, mobile/hosted integration and publication. The default
unit and native fixture suites do not change live accounts or configuration.

For explicit live member checks, `test/miniapp-live-server.mjs` starts a loopback
sidecar using a separately named SQLite file and a real loopback Palpo upstream.
Rinx's `live_palpo.py` uploads only code to mini1, borrows the explicitly supplied
current Rinx session in memory, tests native member submission and role denial,
then stops the sidecar. It does not copy the production database, start notification
workers, or deploy public routes. The 2026-10-03 live member run passed.

Rinx's explicit `live_palpo_admin.py` operator test adds dedicated temporary
admin, owner and bot identities using Palpo's supported `--server false --execute`
CLI. It does not reset existing accounts or restart Palpo. With a private fixture
credential journal passed as the sidecar's sixth argument, it uses real Matrix
authorization, App Service registration and private notification rooms. Only
this test runner accelerates reminder intervals. Its workflow database stays
separate from the production admin store, and its HTTP listener stays on loopback.
Cleanup removes test registrations, leaves test rooms, deactivates test accounts
and verifies their Matrix sessions no longer authenticate.

The live native administrator/owner run on 2026-10-03 passed approval, rejection,
owner-only export authorization, stale decision refusal, notification delivery,
reminders after seen and mini-app disconnect. This verifies the contribution
workflow on real Palpo; it does not claim runtime import, Hagency connection,
project/agent lifecycle or OS push-notification entry acceptance.
