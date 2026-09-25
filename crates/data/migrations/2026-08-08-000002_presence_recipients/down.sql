DROP TABLE remote_presence_recipients;
DROP TABLE presence_recipient_sets;
DROP TABLE presence_recipient_streams;
ALTER TABLE user_presences DROP COLUMN updated_at;
