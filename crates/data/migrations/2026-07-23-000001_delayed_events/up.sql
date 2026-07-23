-- MSC4140 delayed events.
--
-- Scheduled events that the homeserver sends into a room on the user's behalf
-- once their delay elapses. Rows survive restarts so pending events are
-- recovered and sent after the server comes back up. Finalized rows (sent,
-- cancelled, or errored) are retained for lookup and pruned periodically.
CREATE TABLE delayed_events (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    delay_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    device_id TEXT,
    room_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    state_key TEXT,
    content JSONB NOT NULL,
    delay_ms BIGINT NOT NULL,
    txn_id TEXT NOT NULL,
    origin_server_ts BIGINT,
    running_since BIGINT NOT NULL,
    send_at BIGINT NOT NULL,
    event_id TEXT,
    error JSONB,
    finalized_at BIGINT,
    created_at BIGINT NOT NULL,
    UNIQUE (delay_id)
);
-- Idempotency: one delayed event per (user, device session, transaction id).
CREATE UNIQUE INDEX idx_delayed_events_txn ON delayed_events(user_id, COALESCE(device_id, ''), txn_id);
CREATE INDEX idx_delayed_events_user ON delayed_events(user_id);
CREATE INDEX idx_delayed_events_due ON delayed_events(send_at) WHERE finalized_at IS NULL;
CREATE INDEX idx_delayed_events_finalized ON delayed_events(finalized_at) WHERE finalized_at IS NOT NULL;
