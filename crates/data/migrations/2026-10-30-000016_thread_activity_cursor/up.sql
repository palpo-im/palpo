CREATE INDEX threads_room_activity_idx ON threads (room_id, last_sn DESC, event_id DESC);
