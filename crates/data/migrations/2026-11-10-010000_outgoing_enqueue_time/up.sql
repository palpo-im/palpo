-- Existing requests have an unknown enqueue time. A later DEFAULT only applies
-- to new inserts, including old application binaries during a rolling upgrade.
ALTER TABLE outgoing_requests ADD COLUMN enqueued_at BIGINT;
ALTER TABLE outgoing_requests ALTER COLUMN enqueued_at
    SET DEFAULT (floor(extract(epoch FROM clock_timestamp()) * 1000)::BIGINT);
