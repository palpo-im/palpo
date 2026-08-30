CREATE INDEX CONCURRENTLY IF NOT EXISTS e2e_key_changes_room_id_occur_sn_idx
    ON e2e_key_changes USING btree (room_id, occur_sn);
