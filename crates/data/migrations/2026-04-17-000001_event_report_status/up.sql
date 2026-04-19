ALTER TABLE event_reports
ADD COLUMN status TEXT NOT NULL DEFAULT 'new';

CREATE INDEX idx_event_reports_status ON event_reports(status);
