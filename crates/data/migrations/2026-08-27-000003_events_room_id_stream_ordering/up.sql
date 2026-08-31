CREATE INDEX CONCURRENTLY IF NOT EXISTS events_room_id_stream_ordering_idx
    ON events USING btree (room_id, stream_ordering);
