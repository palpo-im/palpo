-- Install the trigger before backfilling, so an older snapshot cannot overwrite
-- a concurrently published receipt. Only trigger installation briefly locks
-- writers; the historical scan runs after that transaction commits.
BEGIN;
CREATE TABLE IF NOT EXISTS event_receipts_notification_cursors (
    user_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    sn BIGINT NOT NULL,
    PRIMARY KEY (user_id, room_id)
);

CREATE OR REPLACE FUNCTION publish_receipt_notification_cursor() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO event_receipts_notification_cursors (user_id, room_id, sn)
    SELECT user_id, room_id, MAX(sn) FROM published_receipts GROUP BY user_id, room_id
    ON CONFLICT (user_id, room_id) DO UPDATE
        SET sn = GREATEST(event_receipts_notification_cursors.sn, EXCLUDED.sn);
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS publish_receipt_notification_cursor ON event_receipts;
CREATE TRIGGER publish_receipt_notification_cursor
    AFTER INSERT ON event_receipts
    REFERENCING NEW TABLE AS published_receipts
    FOR EACH STATEMENT EXECUTE FUNCTION publish_receipt_notification_cursor();
COMMIT;

INSERT INTO event_receipts_notification_cursors (user_id, room_id, sn)
SELECT user_id, room_id, MAX(sn) FROM event_receipts GROUP BY user_id, room_id
ON CONFLICT (user_id, room_id) DO UPDATE
    SET sn = GREATEST(event_receipts_notification_cursors.sn, EXCLUDED.sn);
