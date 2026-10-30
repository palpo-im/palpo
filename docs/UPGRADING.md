# Upgrading Palpo

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
