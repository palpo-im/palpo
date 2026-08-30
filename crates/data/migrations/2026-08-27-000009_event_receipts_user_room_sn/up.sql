CREATE INDEX CONCURRENTLY event_receipts_user_room_sn_idx
    ON event_receipts USING btree (user_id, room_id, event_sn);
