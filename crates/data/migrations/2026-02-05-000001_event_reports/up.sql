-- Event reports table for tracking reported events
CREATE TABLE event_reports (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    received_ts BIGINT NOT NULL,
    room_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    reason TEXT,
    content JSONB,
    score BIGINT
);

-- Index for efficient queries by room
CREATE INDEX idx_event_reports_room_id ON event_reports(room_id);

-- Index for efficient queries by user who made the report
CREATE INDEX idx_event_reports_user_id ON event_reports(user_id);

-- Index for efficient queries by reported event
CREATE INDEX idx_event_reports_event_id ON event_reports(event_id);

-- Index for ordering by received timestamp
CREATE INDEX idx_event_reports_received_ts ON event_reports(received_ts);
