-- Transactional stream SN allocator.
--
-- PostgreSQL sequences are non-transactional: a value can become visible through
-- last_value before the row that uses it commits. The sync token must represent
-- committed stream progress, so allocate stream SNs through a single-row table
-- that is updated inside the caller's transaction.

CREATE TABLE IF NOT EXISTS stream_positions (
    id SMALLINT PRIMARY KEY,
    current_sn BIGINT NOT NULL
);

INSERT INTO stream_positions (id, current_sn)
VALUES (
    1,
    GREATEST(
        COALESCE((SELECT MAX(occur_sn) FROM user_datas), 0),
        COALESCE((SELECT MAX(occur_sn) FROM user_presences), 0),
        COALESCE((SELECT MAX(sn) FROM rooms), 0),
        COALESCE((SELECT MAX(event_sn) FROM event_points), 0),
        COALESCE((SELECT MAX(occur_sn) FROM device_inboxes), 0),
        COALESCE((SELECT MAX(occur_sn) FROM e2e_key_changes), 0),
        COALESCE((SELECT MAX(sn) FROM event_receipts), 0),
        COALESCE((SELECT MAX(occur_sn) FROM room_joined_servers), 0),
        COALESCE((SELECT MAX(occur_sn) FROM room_typings), 0),
        COALESCE((SELECT last_value FROM occur_sn_seq), 0)
    )
)
ON CONFLICT (id) DO UPDATE
SET current_sn = GREATEST(stream_positions.current_sn, EXCLUDED.current_sn);

CREATE OR REPLACE FUNCTION next_stream_sn()
RETURNS BIGINT
LANGUAGE SQL
VOLATILE
AS $$
    UPDATE stream_positions
    SET current_sn = current_sn + 1
    WHERE id = 1
    RETURNING current_sn;
$$;

CREATE INDEX IF NOT EXISTS event_points_event_sn_idx
    ON event_points USING btree (event_sn);

ALTER TABLE user_datas
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
ALTER TABLE user_presences
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
ALTER TABLE rooms
    ALTER COLUMN sn SET DEFAULT next_stream_sn();
ALTER TABLE event_points
    ALTER COLUMN event_sn SET DEFAULT next_stream_sn();
ALTER TABLE device_inboxes
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
ALTER TABLE e2e_key_changes
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
ALTER TABLE event_receipts
    ALTER COLUMN sn SET DEFAULT next_stream_sn();
ALTER TABLE room_joined_servers
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
ALTER TABLE room_typings
    ALTER COLUMN occur_sn SET DEFAULT next_stream_sn();
