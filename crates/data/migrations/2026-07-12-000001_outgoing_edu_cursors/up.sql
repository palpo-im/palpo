-- Per-destination EDU delivery cursor for the federation sender.
--
-- Tracks the last sequence number whose presence/read-receipt/device-list
-- updates were successfully delivered to a remote server, so each transaction
-- can select exactly the EDUs the destination has not seen yet and the
-- position survives restarts.
CREATE TABLE outgoing_edu_cursors (
    server_id TEXT NOT NULL PRIMARY KEY,
    edu_sn BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
