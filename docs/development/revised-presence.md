# Revised social presence (MSC4532)

Build the server with `--features unstable-msc4532` to opt into the experimental
protocol. This also enables MSC4495 selective presence. Presence visibility continues
to follow the user's sharing account data; enabling revised presence does not opt
users into broadcasting to all room members.

The protocol uses the unstable identifiers from
[MSC4532](https://github.com/matrix-org/matrix-spec-proposals/pull/4532).
Clients set availability per device through `/sync` or `PUT /presence/{userId}/status`.
The simplified sliding-sync request body also accepts `set_presence`.
Connected devices resolve in this order: persistent override, busy, active, idle.
If no devices remain connected, the result is always offline. Device activity expires
five minutes after the most recent sync/status request, with a 30-second sweep.
There is no automatic idle transition.

User-controlled overrides and extensible status are stored in global account data
`org.continuwuity.presence_v2.msc4532.presence.persistent`. Writing `{}` or deleting
that event clears the override and status. Unknown override strings are retained but
ignored by state resolution. Unknown status object properties survive forwarding.
Persistent settings survive restart; stale device activity does not keep a user online.
For legacy clients, an explicitly supplied `status_msg` updates only `status.msg`
using a compare-and-swap write, preserving overrides and other status properties.

For revised state responses, pass
`org.continuwuity.presence_v2.msc4532.revised_presence=true` to GET `/sync` or
GET `/presence/{userId}/status`. Without it, active maps to online and idle/busy map
to unavailable. Responses include the extensible status and a matching legacy
`status_msg`. Incoming federation EDUs accept legacy state/activity and status fields,
with the extensible status object taking precedence when present. Outbound federation
uses the revised states and retains the deprecated compatibility fields.

Exclusive application-service users do not have inactivity timeouts. Failed outbound
delivery starts a five-minute debounce; a successful delivery restores their saved
client states. Remote-server failures use the same debounce, publish an offline view
without discarding remote state, and republish the saved state after recovery.
Application-service presence transactions require `receive_ephemeral` and an
authorized recipient in the service's exclusive namespace. Authorization is checked
again before a queued transaction is sent.

Run the state/wire tests (including federation and application-service payload
round trips) with:

```text
cargo test -p palpo-core --features unstable-msc4532 --lib presence::revised
```

The opt-in database regression test requires an **empty dedicated PostgreSQL database**:

```text
PALPO_TEST_DATABASE_URL=postgres://... cargo test -p palpo --lib --features unstable-msc4532 durable_multidevice_overrides_status_and_remote_recovery -- --ignored
```

The `Revised presence federation` CI job builds the Complement image with
`PALPO_FEATURES=unstable-msc4532` and runs the repository's
`tests/complement/revised_presence` overlay against the pinned Complement revision.
It starts two actual homeservers and requires every test to pass in both directions:
revised states and extensible status via incremental sync, legacy-client projection,
status removal, explicit offline, and exclusion of an unauthorized room member.
This job is separate from the default-feature Complement and mixed-Synapse jobs;
those jobs alone do not establish MSC4532 interoperability.
