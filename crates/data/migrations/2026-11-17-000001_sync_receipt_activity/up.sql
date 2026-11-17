CREATE INDEX CONCURRENTLY IF NOT EXISTS event_receipts_public_sync_window
    ON event_receipts (sn, room_id) WHERE ty <> 'm.read.private';
