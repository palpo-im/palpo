-- A scalar watermark cannot page a device-change tie larger than one EDU
-- transaction. Keep a bounded continuation for each selector instead.
CREATE TABLE outgoing_selector_cursors (
    server_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('device', 'receipt')),
    sn BIGINT NOT NULL,
    token TEXT,
    PRIMARY KEY (server_id, kind)
);

CREATE INDEX e2e_key_changes_outgoing_window
    ON e2e_key_changes (room_id, occur_sn, user_id);
CREATE INDEX event_receipts_outgoing_window
    ON event_receipts (room_id, sn) WHERE ty = 'm.read';
