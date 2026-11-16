BEGIN;
DROP TRIGGER IF EXISTS publish_receipt_notification_cursor ON event_receipts;
DROP FUNCTION IF EXISTS publish_receipt_notification_cursor();
DROP TABLE IF EXISTS event_receipts_notification_cursors;
COMMIT;
