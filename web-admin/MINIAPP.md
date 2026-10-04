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
are grandfathered. Without this option, legacy direct project creation remains
enabled; do not describe that deployment as enforcing mandatory project approval.
Projects activated from Inbox always receive a resource grant, enforced on later
agent requests through both frontends. Activation uses the owner's live token.

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

This protocol is **not enabled in the mini app yet**. Contribution capability
publication, explicit project budgets/administrators and Inbox command/result
projection must be wired before a peer advertises support. No implicit unlimited
grant or legacy owner transfer is part of this change.

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

Workflow, audit and notification intent are committed together in SQLite. A
lost send receipt repeats the same Matrix transaction ID. Pending actions get
up to three reminders at one hour, one day and two days after that revision;
failures retry with capped backoff. Reading does not complete the action or stop
reminders. Snooze postpones delivery. A changed revision cancels obsolete notices.
The room's older messages remain history and their links open the latest result.
Quiet hours and the Glance-style room board are not implemented in this revision.

## Validation and release limits

Run Node 24+: `cd web-admin && node --test test/*.test.mjs`.
The native fixture is `test/miniapp-native-fixture.mjs`; Rinx's
`tools/wechat-ux/live/native_palpo.py` starts it, drives the production Splash
frontend through Makepad, and cleans up. HTTP, sessions and SQLite workflows are
real; Matrix/Hagency observations are explicit local test fixtures.

This is an implementation milestone, not completed ADR acceptance. Remaining:
public deployment and native file saving/runtime import, richer My Actions room UI,
Hagency-scoped allocation decisions/top-ups/runtime statistics/revocation,
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
