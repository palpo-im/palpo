CREATE INDEX CONCURRENTLY IF NOT EXISTS room_state_deltas_room_id_idx
    ON room_state_deltas USING btree (room_id);
