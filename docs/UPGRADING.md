# Upgrading Palpo

## Security review: identity bindings, receipts and refresh sessions

Private read receipts are now visible only to their sender in both sync APIs.
Remote media content is limited by `media.max_remote_media_size` even when streamed;
`media.max_concurrent_remote_downloads` (default 16) admits content and thumbnail
fetches without an unbounded waiting queue. A streaming response keeps its slot
until EOF, error or client cancellation. Exceeding the byte budget terminates
the response body; clients must treat a truncated download as failed.

Refresh-enabled native sessions now use `access_token_ttl` (default 60000 ms),
`refresh_token_ttl` for refresh idle expiry (default 120000 ms), and `session_ttl`
for the absolute session deadline (default 3600000 ms). Refresh does not require
an access token. It atomically consumes the refresh credential, rotates both
credentials and preserves the original absolute deadline. A replay is rejected.
Non-refresh logins retain non-expiring access tokens. Existing refresh sessions
receive enforced access expiry on their next successful refresh; upgrading does
not retroactively expire unlinked legacy access tokens. Revoke legacy sessions
explicitly if their provenance or previous lifetime is unacceptable.

The `oidc_identity_bindings` migration creates an empty identity binding table.
**Before enabling OIDC traffic after upgrade**, operators must verify legacy
account ownership using their trusted provider directory and populate the exact
`(issuer, subject, user_id)` mapping in this table in a maintenance window. Use
the issuer string returned by the validated discovery metadata and the original
case-sensitive subject. The database enforces unique external identities and
unique bound accounts. Do not derive mappings from display names, email prefixes
or the previous generated Matrix localpart alone. Retain an audit trail of the
verified mapping and use parameterized SQL in migration tooling.

Unbound identities cannot implicitly claim an existing Matrix ID. With OIDC
registration disabled they are rejected; with registration enabled they receive
a new account, allocating a new localpart if the proposed name already exists.
This includes legacy OIDC accounts that have not been verified and bound. Exact
issuer/subject bindings survive provider username and profile changes. Provider
issuer changes require an explicitly verified administrative migration. There
is no automatic account-linking endpoint or unverified backfill.

## Query-scoped connection leases

Building from source now requires Rust 1.96 or newer for Horme 0.4. Ordinary
queries acquire connections only during execution; transactions and coordination
locks retain their explicit connection scopes. The configured total connection
budget and query/coordination split are unchanged. No database migration or
coordinated deployment is required for this change. See
[database connection lifetimes](development/database.md) for embedding and query
helper guidance.

Embedding code must remove `.await?` from `connect()` and
`PreparedDatabase::connect()`; they now return a lightweight provider directly.
`coordination_connect()` follows the same rule. Pass the handle to Horme queries
or use its transaction methods. Physical-session instrumentation opts into the
explicit `checkout().await` API instead.

## Commit-safe device-list sync positions

Upgrade all homeserver nodes that publish key changes or issue sync cursors
together before relying on the new device-list publication fence. Stop old
writers, deploy the upgraded nodes, and resume traffic through upgraded readers.
An older node does not participate in the fence, even if the device-list journal
migration has already been applied. An old reader can still issue a cursor past
an upgraded writer's uncommitted position; both sides must be upgraded for the
guarantee to hold. This change adds no database migration.

Local device and signing-key notifications now allocate their position and write
all marker scopes in one transaction. Remote device versions and bulk device
revocation participate in the same fence. Classic/sliding sync, invitation cursor
recapture and the shared EDU cursor cannot advance past those uncommitted key
positions. A failed transaction may leave a harmless sequence gap; its journal,
markers and transactional side effects roll back together.

Different users can publish concurrently. Publishers for one user serialize
before taking the shared key-stream lock; cursor captures take its exclusive
form after the other stream locks. Do not add a publisher that allocates its
sequence before this fence, or make a publisher take presence/profile/live/inbox
snapshot locks while holding the key-stream lock.

The fence does not reconstruct notifications skipped before this upgrade.
Clients whose device-list tracking spanned the old publication race should
refresh tracked device lists. Rolling application code back restores the old
race even when the journal table remains present.

## Device-list history migration (2026-11-10)

`2026-11-10-000000_device_key_change_history` adds a transactional journal of
device-key change positions. Its PostgreSQL trigger records insert attempts and
updates to the existing latest-per-room markers, including writes from old
binaries. Recording happens before conflict handling, so a delayed older
notification remains in its historical window while the latest marker stays
newer. Journal records are deduplicated and roll back with the writer transaction.
This migration is additive: deployments already upgraded through the coordinated
2026-10-30 and media-reservation migrations may apply it before rolling out the
new application. Keep the trigger and journal while any upgraded reader runs.
Mixed application versions still give different `/keys/changes` and sync answers
until all readers are upgraded, so route these requests to upgraded nodes if the
new semantics are required during rollout.

The initial backfill preserves markers still present at migration time. Older
updates already overwritten before migration cannot be reconstructed. Clients
resuming pre-upgrade device-list tracking should refresh tracked device lists;
do not treat a bounded historical `changed` result as complete for that period.
Historical `left` is reconstructed from retained immutable live-publication
state frames. Device-list journal positions occupy the half-open interval
`[from, to)` of next-sync tokens, shared by classic and sliding sync.

An application rollback may retain the additive journal and trigger, but restores
the old incomplete `left` behavior. Before explicitly rolling back this database
migration, stop every upgraded reader; it requires the journal table. Existing
requirements to coordinate earlier migrations remain in effect.

## Media reservation migration (2026-11-09)

Stop every old writer before applying
`2026-11-09-000000_media_upload_reservations`, then start upgraded writers
together. Old binaries do not enforce reservation ownership and must not share
the database with upgraded upload handlers. Existing uploaded media remains
readable. Unused MXCs issued before this upgrade have no owner record: clients
must create a new MXC before uploading. New reservations expire after 24 hours;
requests authorized before expiry may finish publishing after expiry.

Client media downloads now proxy S3 objects so the server can enforce safe
Content-Disposition and nosniff headers. The S3 redirect setting still applies
to federation media locations. Plan bandwidth accordingly.

Keep the reservation table when rolling application code forward. A rollback
to a binary without ownership checks reintroduces the security issue; do not
use it as an online rollback while accepting uploads.

## Coordinated upgrade for the 2026-10-30 migrations

Upgrade every Palpo writer sharing a PostgreSQL database together. Do not mix old
and upgraded nodes in a rolling deployment for this release.

1. Stop all old Palpo processes and workers that write to the database **before**
   any node runs `2026-10-30-040000_live_state_publications`.
2. Start one upgraded node with the normal database configuration. Wait for its
   migrations to complete and startup to succeed.
3. Start the remaining upgraded nodes. Keep all old writers stopped.

Upgrade all nodes serving client requests before resuming traffic. Sync now
issues opaque publication cursors, which older binaries cannot interpret.
Existing `s`/`t` history cursors remain supported; clients should store and
replay pagination tokens without interpreting their contents.

The sync endpoint accepts old `s` sync positions. For history requests, old `s`
tokens retain their original identity meaning in `/messages` and `/relations`;
qualified `q` pages retain each original range independently. The old `s`
format does not identify which endpoint issued it. Exact reconstruction of
late-published history across an upgrade may therefore require a full sync when
resuming an old sync boundary. Normal clients do not need to discard otherwise
valid history cursors.

The publication migration backfills metadata only for accepted live rows with
saved state when the migration runs. Old writers started afterward would create
rows without the publication metadata required by upgraded sync readers. The
backfill cannot reconstruct publication order already recorded incorrectly by
older versions.

Large to-device batches also require every writer to use the upgraded shared
inbox fence; the coordinated sequence above covers this requirement. A legacy
to-device receipt may produce one duplicate delivery when first retried after
the upgrade.
