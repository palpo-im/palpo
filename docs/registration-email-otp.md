# Registration email verification with AgentMail

Palpo can require a verified email before creating a local human account. Rinx
performs the complete flow in its shared desktop/mobile login screen. AgentMail
only delivers email: Palpo generates, checks, expires and consumes each proof.

## Server setup

Create an AgentMail account, complete its human verification, and obtain a sender
inbox and API key. Store the API key and an independent random OTP signing secret
in private files readable only by the Palpo service user. Each secret must contain
at least 32 characters. Keep the OTP secret stable across restarts and replicas.
Never commit these files or configure the key in Rinx.

```toml
allow_registration = true
# Existing invitation requirements remain in force when configured.
# registration_token = "your-invitation-token"

[registration_email]
agentmail_inbox = "your-inbox@agentmail.to"
agentmail_api_key_file = "/private/palpo/agentmail-api-key"
otp_secret_file = "/private/palpo/email-otp-secret"
```

The presence of this section makes email mandatory for local human registration.
Removing it restores the prior registration policy. Apply the migration and
restart Palpo normally; secrets are validated at startup. Configure
`well_known.client` with the public client API origin, including a nonstandard
port if needed. A provider outage fails closed for new registrations; existing
accounts and sign-in are unaffected. Trusted appservice provisioning remains
available; guest registration cannot bypass the email policy. Delegated browser
registration retains its existing provider-controlled verification.

The default transport base URL is `https://api.agentmail.to/v0/`. The
`agentmail_api_url` override accepts HTTPS, or HTTP only on loopback for tests.
Outbound credentials are never forwarded across redirects. AgentMail onboarding
and sending limits still apply; monitor rejected sends in Palpo's server logs.

## Protocol and limits

- `GET /_matrix/client/versions` advertises
  `org.palpo.registration.email_otp`; the Palpo discovery endpoint is
  `GET /_matrix/client/unstable/org.palpo.registration`.
- `POST /_matrix/client/v3/register/email/requestToken` accepts `email`,
  `client_secret` and `send_attempt`, and returns `sid` and `submit_url`.
- `POST /_matrix/client/v3/register/email/submitToken` accepts `sid`,
  `client_secret` and the six-digit `token`, and returns `success: true`.
- Registration uses standard `m.login.email.identity` UIAA with `threepid_creds`
  (`sid`, `client_secret`) and the server-issued UIAA `session`. An invitation
  token, when configured, is an additional UIAA stage.

Codes expire after 10 minutes and lock after five wrong guesses. Resends wait
60 seconds and invalidate earlier codes from that client. A recipient can receive
at most six sends per hour, including failed-delivery reservations. Per-IP send
and verify limits supplement the durable recipient and attempt limits. Configure
`trusted_proxies` only for actual trusted proxies, never arbitrary clients.

Verified proofs expire after 30 minutes, bind to one UIAA session and username,
and are consumed atomically with the new account, password and verified email.
Concurrent registration cannot overwrite an existing user's password. The verified
email is visible through `GET /account/3pid`; identity-server publishing and contact
changes after registration are still unsupported. This change does not implement
email password recovery or the separate administrator-approval workflow.

## Validation

```sh
docker run -d --name palpo-email-otp-db \
  -e POSTGRES_PASSWORD=palpo-otp-isolated-test \
  -p 127.0.0.1:54329:5432 postgres:18
cargo build --locked -p palpo
python3 tests/registration_email_otp.py \
  --binary target/debug/palpo --output target/email-otp-test --serve
```

The test creates its own database and uses a recording AgentMail transport. It
checks delivery failure, idempotency, cooldown, incorrect/expired/locked codes,
resend invalidation, invitation stages, username binding, stored contacts, replay
and concurrent account creation. `--serve` keeps it alive for Rinx's
`tools/wechat-ux/live/native_email_registration.py`; pass the generated `state.json`.
Stop the script after native validation; it drops only its own database. Remove
the test container when finished. No real email is sent by these tests.

AgentMail API: https://docs.agentmail.to/api-reference/inboxes/messages/send

Acceptance on 2026-10-07: the server built successfully, both email configuration/
digest unit tests passed, the Matrix `threepids` wire-format regression passed,
and all ten black-box PostgreSQL checks passed. A verified AgentMail inbox also
accepted a real Palpo verification email; recipient-code confirmation is separate
from transport acceptance. Native desktop and phone-size Rinx account creation
passed against the recording transport.
