CREATE INDEX CONCURRENTLY IF NOT EXISTS event_points_room_id_event_sn_idx
    ON event_points USING btree (room_id, event_sn);
