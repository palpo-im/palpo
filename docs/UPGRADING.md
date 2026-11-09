# Upgrading Palpo

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
