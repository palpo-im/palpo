# Incremental sync work for filtered rooms

Incremental v3 sync captures current and previous room frames in batches. When
all four room sections exclude every event type, an unchanged frame needs no
per-room loader if there is no fresh receipt/notification update or deliverable
sticky event. Receipt watermarks and sticky candidate rooms are read in batches
of at most 256 room IDs. A transactional projection stores one receipt watermark
per user/room; sync never aggregates the append-only history. The migration
installs its insert trigger before backfilling existing rows, and `GREATEST`
keeps concurrent or out-of-order positions monotonic. The backfill runs outside
the trigger-installation transaction so it does not hold that writer lock.
Room-ID exclusions can also skip an unchanged frame.
Changed membership frames still feed independent device and presence streams.
Initial sync, `full_state`, peeks and requested thread-notification maps keep
their existing paths.

The watermark is the receipt publication position, rather than the position of
the message it marks as read. Reading an older message must still cause updated
notification counts. Query errors propagate instead of acknowledging an empty
batch. Receipt insertions allocate positions under a shared transaction advisory
lock; sync snapshots take the exclusive lock first and wait for publication to
commit or roll back. Client receipt insertion and notification clearing retain
one commit point. No federation request runs while holding this transaction.

The isolated regression exercises the loader bypass, unchanged/changed frames,
full-state fallback, sticky delivery, current and older-event receipts, another
user's receipt, bounded queries for over 1,000 IDs and database-error recovery.
With 30,000 historical receipts, the actual query plan reads only the small
projection. Out-of-order inserts cannot regress its watermark.
An actual PostgreSQL lock waiter proves that snapshots wait for both committed
and rolled-back publications.

This addresses the fully event-filtered bridge path in
[#650](https://github.com/palpo-im/palpo/issues/650) and batches notification
watermarks for ordinary incremental syncs. The issue remains open for a general
activity prefilter for clients requesting room payloads. That prefilter must
account for live publications, account data, public/private receipts, typing
expiry and sticky delivery, while preserving independent membership/device
streams; a timeline-only room index would miss updates.
