# Message retention (MSC1763)

Build with `--features unstable-msc1763` and explicitly enable retention:

```toml
[retention]
enable = true

[retention.policies."*"]
max_lifetime = 2592000000 # 30 days, in milliseconds

[retention.policies."!room:example.org"]
max_lifetime = 604800000 # override this room: 7 days

[retention.limits.max_lifetime]
min = 3600000
max = 7776000000
```

The feature is disabled by default. Enabled builds advertise
`org.matrix.msc1763` in `/versions`. Authenticated clients can query
`GET /_matrix/client/unstable/org.matrix.msc1763/retention/configuration`.
The response contains the operator's `policies` and `limits`; `*` denotes the
default. Per-room operator overrides are returned only to joined users.
Disabled configurations return 404 and do not advertise support.

Room administrators can set `org.matrix.msc1763.retention` with the empty state
key and ordinary state-event power levels. `min_lifetime` and `max_lifetime`
are nullable/optional integer millisecond lifetimes in `[0, 2^53 - 1]`.
When both are supplied, the minimum must not exceed the maximum. Local invalid
policies and nonempty state keys are rejected before publication. An invalid
remote policy is treated as an empty room policy, subject to operator limits;
it does not prevent room operation.

## Effective policy

An operator per-room override takes precedence. Otherwise, a room's current
retention state event replaces the operator default. If no such state event
exists, the default applies, or retention is unlimited when no default exists.
An explicitly empty room policy therefore does not inherit default lifetimes.

Server limits clamp each room lifetime independently. An omitted room property
uses the corresponding limit's minimum, if supplied. An omitted maximum with no
such minimum remains unlimited. Limits apply to room policies; operator
defaults/overrides are validated at startup: their supplied values must comply
with the limits, while omitted operator properties remain omitted.
If clamping crosses the two lifetimes, the maximum lifetime takes precedence
unless the operator's minimum-lifetime floor requires raising both values.
Incompatible floors/ceilings are rejected. The final policy always preserves
minimum <= maximum. A minimum alone does not schedule any deletion.

The latest policy applies to existing messages as well as new ones. Expiry is
measured from `origin_server_ts`, including equality at the maximum lifetime.
Future timestamps do not wrap or expire early. Lifetime zero expires eligible
messages immediately.

## Local guarantees

Only non-state event payloads expire. State, authorization chains and event
graph records are retained, so retention does not remove room authorization or
break state resolution. Every current forward extremity is exempt until a
new event references it; this preserves the proposal's latest-event exemption
across forks. Consequently, an otherwise expired last message can remain
visible while its room is inactive.

Shared PDU loaders apply retention before returning payloads. Client event
lookup and context refuse expired events; stream/topological timelines used by
messages and both sync implementations omit them. Search checks event visibility
and removes expired search index entries. Sticky sync also applies retention
before its history-visibility exemption. Shared federation loaders return only
skeletal PDUs for expired messages. Their hashes, signatures and auth/prev
references are retained; arbitrary content and unsigned bundles are removed.
Administrative PDU lookups use the same loaders.

Expiry is permanent once observed: a persisted marker prevents content from
returning after a longer policy, a restart, or federation backfill. Database
guards also prevent re-indexing a purged message. Payload removal and deletion
of search, relation, sticky and per-event push indexes happen transactionally.
Finalized delayed-send records lose their duplicate content as well; a late
finalization cannot restore it. Their scheduling/idempotency metadata remains.
The background worker scans persisted events on startup and every 60 seconds;
it has no in-memory schedule to lose during a restart. Read checks close the
window before a sweep. Failed purge operations are retried on the next sweep
and reads fail closed if they cannot enforce expiry.

This experimental implementation retains essential event metadata rather than
deleting all references to the event as the proposal describes. It does not
emit a room redaction event or force other homeservers/clients to erase their
copies. Media files, client caches, exports and operator backups have separate
lifecycles. Physical PostgreSQL space reclamation also depends on vacuuming.
Operators must enable the feature consistently on all nodes sharing a database;
expiry markers and database content guards remain permanent even after disabling
the feature. Event timestamps are asserted by sending servers, so retention
cannot certify a remote event's real age.

## Validation

Unit fixtures cover lifetime syntax/range/order, policy precedence, empty room
policies, server limits, equality and future timestamps. The HTTP/PostgreSQL
regression creates a real room and exercises event lookup, messages, context,
both sync APIs, search, shared federation loaders, policy changes, terminal/state
preservation, backfill/re-indexing protection, a restart in a separate process,
repeated sweeps and subsequent room operation. A separate test verifies disabled
configuration and advertisement.

Run each ignored test alone in a fresh **empty dedicated PostgreSQL database**:

```sh
cargo test -p palpo-core --features unstable-msc1763 retention
cargo test -p palpo --lib --features unstable-msc1763 retention
cargo test -p palpo --lib --features unstable-msc1763 retention_http_expiry -- --ignored
cargo test -p palpo --lib --features unstable-msc1763 retention_http_disabled -- --ignored
```

Set `PALPO_TEST_DATABASE_URL` for the last two commands. Tests refuse existing
databases. CI runs the two HTTP fixtures in separate databases.
