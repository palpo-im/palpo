CREATE INDEX CONCURRENTLY IF NOT EXISTS room_typings_sync_window
    ON room_typings (occur_sn, room_id);
