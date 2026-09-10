# Account requests and Matrix administrator approval

The signup page uses the same Palpo Matrix accounts as the web member dashboard
and Robrix. A request does not create an account, a project, or an Agent. It
queues a private administrator decision. Hagency resource approval is unchanged.

1. Applicant chooses **Request an account**, a local username, display name,
   password (12–256 characters), and a reason. A private status receipt is stored
   in this browser; the password is not stored in browser storage.
2. The applicant sees **Waiting for administrator approval**. Reloading the page
   and reopening the request restores its status.
3. The account worker posts an `org.octos.approval_request` with **Approve** and
   **Reject** actions in **Palpo · Account approvals**. No password, browser
   receipt, registration token, or service credential is included.
4. A configured server administrator accepts the room invitation in Robrix and
   clicks an action. Existing Robrix Octos approval rendering and its typed
   response support this flow; no new Robrix executable is required.
5. Palpo Web reads the original Matrix events from that room, checks the exact
   request/event/digest/reply binding, current membership, current server-admin
   authority and deadline. Text replies do not approve requests.
6. An approved request creates an ordinary local Matrix account using the
   homeserver registration API. The applicant signs in with their chosen
   password, creates a project and submits an Agent request to Hagency.

## Operator configuration

Set `PALPO_ACCOUNT_CONFIG` to a private JSON file outside the release. The Compose
deployment supports `/app/data/account-approval.json` in its existing named
volume. It must be readable by container user `node` (uid1000) and mode0600.

```json
{
  "botMxid": "@account_requests:example.org",
  "botToken": "<ordinary dedicated account token>",
  "adminToken": "<dedicated server-side administrator token>",
  "approvers": ["@administrator:example.org"],
  "registrationToken": "<configured homeserver registration token>",
  "passwordKey": "<64 random lowercase hexadecimal characters>"
}
```

The bot is an ordinary account, distinct from human approvers. The privileged
token is used only on the configured homeserver; it verifies administrators and
registration ownership. It is not sent to a browser or Matrix room. Passwords
are encrypted with AES-256-GCM, binding ciphertext to its request ID; the key
belongs in the private operator file. Password records are removed on confirmed
registration, rejection, expiry or username conflict. Keep database backups and
the configuration protected. Never log the configuration or request bodies.

The worker creates an invite-only, non-federated **group** room, with history
visible from the administrator's invitation, including requests sent before they
accept that invitation. It is not a DM and does not contain passwords. This
HTTP worker does not decrypt Matrix rooms: enabling room encryption or admitting
an unrelated account stops processing with an explicit admin-visible error.
Current configured approvers must retain local server-admin status; room power
levels alone do not authorize account creation. Adding administrators or changing
the bot/key needs an explicit pending-request migration; identity/key binding
prevents silently adopting another worker's requests.

Requests expire after seven days. Decisions, original event IDs, message cursor,
registration attempt identity and status persist in SQLite. Notifications use
stable transaction IDs. Lost registration responses are recovered only if Palpo
reports the exact random device ID chosen for that request; an existing account
is never adopted, overwritten or password-reset. Row retries back off up to one
minute. Request endpoints have same-origin checks, bounded payloads/queues and
rate limits; status requires the unguessable receipt. The browser retains its
receipt during network failures.

No configuration means signup is disabled, preserving login-only installations.
An invalid configured identity or room keeps signup unavailable and exposes the
problem to logged-in administrators. Account processing uses its own worker and
does not hold Hagency's mutation queue during polling or registration.

## Verification

`npm test` includes authorization, replay, conflicts, expiry, password handling,
restart recovery and real local HTTP boundary tests. `node test/accounts.browser.mjs`
uses Playwright against a fixture homeserver for signup, approval receipt, login,
project creation and an Agent request that stays pending Hagency approval. Its
Matrix decision is a fixture, not a native Robrix click. Real homeserver/Robrix evidence
must be recorded separately.
