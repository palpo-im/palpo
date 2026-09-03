-- Durable handoff from delayed-event completion to the federation queue.
CREATE TABLE delayed_event_deliveries (
    event_id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL
);
-- Existing completed events may have stopped before queueing. Re-delivery is
-- idempotent by event ID; a missed delivery would otherwise be permanent.
INSERT INTO delayed_event_deliveries (event_id, room_id)
SELECT event_id, room_id FROM delayed_events WHERE event_id IS NOT NULL
ON CONFLICT DO NOTHING;
