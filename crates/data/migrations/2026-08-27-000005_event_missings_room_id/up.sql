CREATE INDEX CONCURRENTLY IF NOT EXISTS event_missings_room_id_idx
    ON event_missings USING btree (room_id);
