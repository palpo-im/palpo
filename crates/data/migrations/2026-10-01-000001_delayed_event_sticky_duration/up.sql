ALTER TABLE delayed_events
    ADD COLUMN sticky_duration_ms INTEGER
    CHECK (sticky_duration_ms BETWEEN 0 AND 3600000);
