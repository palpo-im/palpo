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

-- The timeline append and the delayed-event outcome are written through
-- different connections. Record the output in the same transaction that
-- promotes the event from an outlier into the timeline, so a process crash
-- between append and outcome finalization cannot cause a second room event on
-- recovery. The primary key also rejects any accidental second append for the
-- same delay id.
CREATE TABLE delayed_event_outputs (
    delay_id TEXT NOT NULL PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE
);

-- Remember the candidate as soon as its outlier is persisted. If an attempt
-- fails before promotion, a later attempt may replace that still-outlier
-- candidate. Once the mapped event is promoted, this mapping is immutable.
CREATE FUNCTION palpo_track_delayed_event_output() RETURNS TRIGGER AS $$
DECLARE
    v_delayed_id TEXT;
BEGIN
    v_delayed_id := NEW.json_data -> 'unsigned' ->> 'org.matrix.msc4140.delay_id';
    IF v_delayed_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM delayed_events
        WHERE delay_id = v_delayed_id
          AND user_id = NEW.json_data ->> 'sender'
          AND room_id = NEW.room_id
          AND finalized_at IS NULL
    ) THEN
        INSERT INTO delayed_event_outputs (delay_id, event_id)
        VALUES (v_delayed_id, NEW.event_id)
        ON CONFLICT (delay_id) DO UPDATE
        SET event_id = EXCLUDED.event_id
        WHERE EXISTS (
            SELECT 1 FROM events
            WHERE id = delayed_event_outputs.event_id
              AND is_outlier = TRUE
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER event_datas_track_delayed_event_output
AFTER INSERT ON event_datas
FOR EACH ROW EXECUTE FUNCTION palpo_track_delayed_event_output();

-- Promotion is the definitive commit point. A different event already mapped
-- to this delay id must make this promotion fail, otherwise two events could
-- become visible after recovery from a partially completed append.
CREATE FUNCTION palpo_confirm_delayed_event_output() RETURNS TRIGGER AS $$
DECLARE
    v_delayed_id TEXT;
    v_mapped_event_id TEXT;
BEGIN
    SELECT json_data -> 'unsigned' ->> 'org.matrix.msc4140.delay_id'
    INTO v_delayed_id
    FROM event_datas
    WHERE event_id = NEW.id;
    IF v_delayed_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM delayed_events
        WHERE delay_id = v_delayed_id
          AND user_id = NEW.sender_id
          AND room_id = NEW.room_id
          AND finalized_at IS NULL
    ) THEN
        SELECT output.event_id INTO v_mapped_event_id
        FROM delayed_event_outputs AS output
        WHERE output.delay_id = v_delayed_id;

        IF v_mapped_event_id IS NULL THEN
            INSERT INTO delayed_event_outputs (delay_id, event_id)
            VALUES (v_delayed_id, NEW.id);
        ELSIF v_mapped_event_id <> NEW.id THEN
            RAISE EXCEPTION 'delay id % is already mapped to event %',
                v_delayed_id, v_mapped_event_id
                USING ERRCODE = 'unique_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_record_delayed_event_output
AFTER UPDATE OF is_outlier ON events
FOR EACH ROW
WHEN (OLD.is_outlier IS DISTINCT FROM FALSE AND NEW.is_outlier = FALSE)
EXECUTE FUNCTION palpo_confirm_delayed_event_output();

-- Outcome markers only need to live as long as their delayed-event row.
CREATE FUNCTION palpo_remove_delayed_event_output() RETURNS TRIGGER AS $$
BEGIN
    DELETE FROM delayed_event_outputs WHERE delay_id = OLD.delay_id;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER delayed_events_remove_output
AFTER DELETE ON delayed_events
FOR EACH ROW EXECUTE FUNCTION palpo_remove_delayed_event_output();
