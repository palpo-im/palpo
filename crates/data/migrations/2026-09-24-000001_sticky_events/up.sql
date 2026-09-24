-- Sticky events (MSC4354).
--
-- A sticky event must reach every joined client regardless of the sync
-- `timeline_limit`, and must stop being delivered once it expires. The
-- stickiness itself lives in the PDU (`msc4354_sticky.duration_ms`), but
-- scanning event JSON to find the handful of unexpired sticky events in a room
-- on every sync is not viable, so the derived expiry is indexed here.
--
-- `expires_at` is `min(received_at, origin_server_ts) + min(duration_ms,
-- 3600000)` as milliseconds since the unix epoch, computed once when the event
-- is persisted. Storing the absolute instant rather than the duration is what
-- makes expiry survive a restart: nothing is recomputed from process uptime, so
-- an event that expired while the server was down stays expired.
-- `deliver_sn` is the sync position the event is delivered at, assigned when it
-- reaches the timeline. It is not `event_sn`: a federated event can be stored as
-- an outlier and promoted much later, by which time clients have synced well past
-- the position it was given on arrival and would never see it.
CREATE TABLE event_stickies (
    event_id TEXT NOT NULL PRIMARY KEY,
    event_sn BIGINT NOT NULL,
    deliver_sn BIGINT,
    room_id TEXT NOT NULL,
    expires_at BIGINT NOT NULL
);

-- Finding the unexpired sticky events of a room, either in full (a user who
-- just joined) or since a sync position (an incremental sync).
CREATE INDEX event_stickies_room_expires_idx ON event_stickies (room_id, expires_at);
CREATE INDEX event_stickies_room_sn_idx ON event_stickies (room_id, deliver_sn);

-- Reaping expired rows across all rooms.
CREATE INDEX event_stickies_expires_idx ON event_stickies (expires_at);
