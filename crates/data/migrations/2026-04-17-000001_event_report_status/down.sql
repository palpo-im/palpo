DROP INDEX IF EXISTS idx_event_reports_status;

ALTER TABLE event_reports
DROP COLUMN IF EXISTS status;
