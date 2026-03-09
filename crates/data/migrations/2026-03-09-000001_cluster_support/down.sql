DROP TABLE IF EXISTS room_typings;
DROP TABLE IF EXISTS sliding_sync_connections;
ALTER TABLE user_uiaa_datas DROP COLUMN IF EXISTS request_body;
ALTER TABLE outgoing_requests DROP COLUMN IF EXISTS retry_count;
ALTER TABLE outgoing_requests DROP COLUMN IF EXISTS last_failed_at;
