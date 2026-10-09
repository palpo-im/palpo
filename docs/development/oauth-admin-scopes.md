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
authentication-age, assurance-level and JSON step-up fields are tracked in
issue #487 and are not implemented by this feature.

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

## Administration discovery (MSC4540)

Build with `--features unstable-msc4540` (which also enables MSC4484) to expose
`org.continuwuity.msc4540.admin` in authenticated capabilities responses.
`allowed_scopes` contains the experimental server-administration scope for
provisioned administrators, and is empty for other users. It describes scopes
the user may request, independently of the current OAuth token's grants.
Native administrators receive the same list and retain legacy access.

The existing `m.account_moderation` capability is retained. Discovery never
grants privileges: administration routes continue to check both administrator
status and, for OAuth tokens, the management scope. Administrator policy changes
are reflected by subsequent authenticated requests. Builds without MSC4540 omit
the capability. MSC4540 does not add a `/versions` flag.

The current scope challenge is usable before MSC4363 support is added by #487;
clients must obtain the advertised scope from the delegated authorization
service before performing administrative operations. The HTTP regression test
also covers discovery before/after scope acquisition and privilege revocation.
