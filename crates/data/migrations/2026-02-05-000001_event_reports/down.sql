-- Rollback event_reports table
DROP INDEX IF EXISTS idx_event_reports_received_ts;
DROP INDEX IF EXISTS idx_event_reports_event_id;
DROP INDEX IF EXISTS idx_event_reports_user_id;
DROP INDEX IF EXISTS idx_event_reports_room_id;
DROP TABLE IF EXISTS event_reports;
