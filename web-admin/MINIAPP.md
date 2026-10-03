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

Set `PALPO_PROJECT_APPROVAL_REQUIRED=1` to require approval for every **new**
project, including requests through the existing browser/API. Existing projects
are grandfathered. Without this option, legacy direct project creation remains
enabled; do not describe that deployment as enforcing mandatory project approval.
Projects activated from Inbox always receive a resource grant, enforced on later
agent requests through both frontends. Activation uses the owner's live token.

Contribution approval records the decision before attempting registration.
Installation failures stay retryable. Configuration delivery is owner-only;
Rinx sends it directly to native file saving and gives the script only a saved/
cancelled result. No configuration credentials enter action cards or notifications.

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
Approvers are configured notification recipients; current server-admin authority
still controls decisions and is checked again before sending admin notifications.

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
real two-account deployment and owner file handoff, richer My Actions room UI,
Hagency-scoped allocation decisions/top-ups/runtime statistics/revocation,
signup decision parity, mobile/hosted integration and publication. No live
server configuration or accounts are changed by the tests.
