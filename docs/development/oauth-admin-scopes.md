# OAuth administration scopes (MSC4484)

Build with `--features unstable-msc4484` to require the experimental
`urn:matrix:client:cc.c10y.msc4484.server_administration` scope on delegated
OAuth tokens used for server administration. Such builds advertise
`org.continuwuity.msc4484.unstable` in `/_matrix/client/versions`.

This policy covers whois and GET/PUT lock/suspend under every stable client
prefix (`v1`, `v3`, `r0`), the `uk.timedout.msc4323` unstable aliases, and all
Palpo-specific and Synapse-compatible administrative routes under
`/_palpo/admin` and `/_synapse/admin`. All also require the provisioned user
to be an administrator. A management scope cannot grant administrator status.
The scope spelling is experimental; the proposed stable spelling is not accepted.

These routes do not require the general Matrix API scope. Delegated tokens
still need a unique Matrix device scope matching a provisioned device, just as
ordinary delegated authentication does. Ordinary client routes continue to
require the stable or MSC2967 Matrix API scope. Scope strings are validated
against RFC 6749, and the authenticated context retains the resulting scope set.

A valid administrator identity lacking the management scope receives HTTP 401,
`M_FORBIDDEN`, and a `WWW-Authenticate: Bearer error="insufficient_scope"`
challenge naming the required scope. Invalid/inactive tokens receive the usual
`M_UNKNOWN_TOKEN`; non-administrators receive `M_FORBIDDEN` without a scope
challenge. This reuses Palpo's existing insufficient-scope response. MSC4363's
authentication-age, assurance-level and JSON step-up fields can be enabled
separately with `unstable-msc4363`, as described below.

Native access tokens and application-service tokens keep their existing
permissions, including native self-whois. Shared-secret MAS provisioning routes
under `/_palpo/mas` and `/_synapse/mas` keep their separate secret authentication.
Without the Cargo feature, delegated administration retains the general API
scope requirement and the feature flag is absent.

## Regression tests

Unit tests run with `cargo test -p palpo --lib`, with and without the feature.
The HTTP/database regression test starts a local mock introspection server and
checks all route aliases, ordinary API access, native tokens, invalid scopes,
non-administrator identities, and both successful and rejected state changes.

Set `PALPO_TEST_DATABASE_URL` to an **empty dedicated PostgreSQL database** and
run this test alone (it refuses databases containing tables):

```sh
cargo test -p palpo --lib --features unstable-msc4484 oauth_admin_routes -- --ignored
```

Use a fresh empty database to repeat the command without `--features`.

## OAuth step-up authentication (MSC4363)

Build with `--features unstable-msc4363` to enable JSON step-up challenges;
this also enables MSC4484's endpoint scopes. Verified, provisioned OAuth
administrators receive HTTP 401 and
`org.matrix.msc4363.M_INSUFFICIENT_USER_AUTHENTICATION` when their token does
not meet an administrative endpoint's scope, freshness, or assurance policy.
Ordinary API requests, invalid tokens, non-administrators, native tokens and
application services retain their existing behavior. Builds without MSC4363
continue to use MSC4484's `WWW-Authenticate` scope challenge.

Optional operator requirements apply to all administrative routes covered above:

```toml
[delegated_auth]
admin_max_age = 300
admin_acr_values = "urn:example:mfa urn:example:hardware-key"
```

The trusted introspection service must supply `auth_time` (Unix seconds of the
active authentication event) and `acr` (one ACR value). An ACR matches any value
in the configured preference list. Absent evidence, future authentication times,
stale authentication, and unmatched ACRs fail closed when that requirement is
configured. With both settings absent, administrative scope is the only added
requirement. These settings do not replace the user's administrator policy.

Challenges use the proposal's `org.matrix.msc4363.acr_values`,
`org.matrix.msc4363.max_age` and `org.matrix.msc4363.scope` keys. ACRs and
scopes are validated space-separated strings; age is an integer in seconds.
The full required scope set includes administration and the verified device;
existing general API access is also preserved for the replacement token.
The error model accepts the proposed stable error code and field aliases, but
serializes the experimental spellings. No `/versions` flag is specified.

Freshness/assurance-protected requests bypass introspection caching. Age is
always measured from `auth_time`, never cache insertion time. Introspection
`exp`, when supplied, also bounds cached token validity. The authorization
service remains responsible for issuing elevated tokens, token rotation and
appropriate lifetimes; Palpo verifies the evidence on each protected request.

Run `cargo test -p palpo-core --features unstable-msc4363 step_up` for wire
fixtures and `cargo test -p palpo --lib --features unstable-msc4363 step_up`
for policy tests. The `oauth_step_up_routes` ignored test uses an empty dedicated
database and a real mock HTTP issuer to exercise challenges, cache bypass and a
successful state-changing retry. Run it alone with `PALPO_TEST_DATABASE_URL`
set, then run `oauth_admin_routes` in a fresh database with the feature enabled
and disabled to check compatibility.
