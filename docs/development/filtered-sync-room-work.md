# Incremental sync room activity

Incremental v3 sync captures current and previous room frames and live
publication positions in batches. An unchanged frame needs no per-room loader
when there is no newer live publication, receipt/notification update, typing,
room account data or deliverable sticky event. Candidate rooms are read in batches
of at most 256 room IDs. A transactional projection stores one receipt watermark
per user/room; sync never aggregates the append-only history. The migration
installs its insert trigger before backfilling existing rows, and `GREATEST`
keeps concurrent or out-of-order positions monotonic. The backfill runs outside
the trigger-installation transaction so it does not hold that writer lock.
The statement trigger groups only newly inserted rows, updating each watermark
once per batch without rescanning history or repeatedly rewriting one cursor.
Room-ID exclusions can also skip an unchanged frame.
Changed membership frames still feed independent device and presence streams.
Initial sync, `full_state`, peeks and requested thread-notification maps keep
their existing paths.

The watermark is the receipt publication position, rather than the position of
the message it marks as read. Reading an older message must still cause updated
notification counts. Query errors propagate instead of acknowledging an empty
batch. Receipt insertions allocate positions under a shared transaction advisory
lock; sync snapshots take the exclusive lock last and wait for publication to
commit or roll back. Client receipt insertion and notification clearing retain
one commit point. No federation request runs while holding this transaction.
Receipt writers take their notification-summary locks before the publication
fence. A snapshot waiting for one device's inbox therefore cannot hold the
receipt fence and block another device's independent snapshot.

The isolated regressions exercise the loader bypass, unchanged/changed frames,
full-state fallback, sticky delivery, current and older-event receipts, another
user's receipt, bounded queries for over 1,000 IDs and database-error recovery.
With 30,000 historical receipts, the actual query plan reads only the small
projection. Out-of-order inserts cannot regress its watermark.
An actual PostgreSQL lock waiter proves that snapshots wait for both committed
and rolled-back publications.

The same activity prefilter serves ordinary clients and fully filtered bridges
in [#650](https://github.com/palpo-im/palpo/issues/650). Even a zero timeline limit
or an empty event-type filter can carry gap metadata, so a new live publication
always retains the room loader. Frame changes retain membership/device/presence
work independently of room exclusions. Skipped visible rooms still confirm lazy
member delivery for the acknowledged position.

Public receipt and typing candidates use recent publication windows with
supporting indexes; room account data reuses the existing `(user_id, occur_sn)`
index. Another user's private receipts do not contribute, while the caller's
private read progress is covered by its notification watermark. Account-data
deletion tombstones and typing expiry publications count as activity. Candidate
reads retain the same unbounded upper position as their auxiliary payload
readers; a later write can cause harmless extra work, but cannot conceal an
update in the captured window. Empty sections skip their candidate queries.
Failures abort the response so the caller can retry the same position.

This removes sequential room payload loads for quiet rooms, rather than making
the whole sync constant-time: joined-room enumeration, captured frame batches
and independent user/device/presence streams still run. Initial sync, full-state
requests, peeks and requested per-thread notification maps remain full paths.
