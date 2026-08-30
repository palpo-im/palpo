CREATE INDEX CONCURRENTLY IF NOT EXISTS event_datas_room_id_idx
    ON event_datas USING btree (room_id);
