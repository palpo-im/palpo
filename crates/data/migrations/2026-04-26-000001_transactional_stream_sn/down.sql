DROP INDEX IF EXISTS event_points_event_sn_idx;

ALTER TABLE room_typings
    ALTER COLUMN occur_sn SET DEFAULT 0;
ALTER TABLE room_joined_servers
    ALTER COLUMN occur_sn SET DEFAULT nextval('occur_sn_seq');
ALTER TABLE event_receipts
    ALTER COLUMN sn DROP DEFAULT;
ALTER TABLE e2e_key_changes
    ALTER COLUMN occur_sn DROP DEFAULT;
ALTER TABLE device_inboxes
    ALTER COLUMN occur_sn SET DEFAULT nextval('occur_sn_seq');
ALTER TABLE event_points
    ALTER COLUMN event_sn SET DEFAULT nextval('occur_sn_seq');
ALTER TABLE rooms
    ALTER COLUMN sn SET DEFAULT nextval('occur_sn_seq');
ALTER TABLE user_presences
    ALTER COLUMN occur_sn SET DEFAULT nextval('occur_sn_seq');
ALTER TABLE user_datas
    ALTER COLUMN occur_sn SET DEFAULT nextval('occur_sn_seq');

DROP FUNCTION IF EXISTS next_stream_sn();
DROP TABLE IF EXISTS stream_positions;
