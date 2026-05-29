-- MSC2444 federated peeking.

-- Remote servers that are actively peeking one of OUR rooms (resident side).
-- We deliver new room events to these servers until the peek expires (unless
-- renewed) or is cancelled.
CREATE TABLE room_peeking_servers (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    room_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    peek_id TEXT NOT NULL,
    renew_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (room_id, server_id, peek_id)
);
CREATE INDEX idx_room_peeking_servers_room ON room_peeking_servers(room_id);
CREATE INDEX idx_room_peeking_servers_renew ON room_peeking_servers(renew_at);

-- Outbound peeks WE hold on remote rooms (peeking side). One active peek per
-- room for this server; local users share it. We renew before renew_at.
CREATE TABLE room_peeks (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    room_id TEXT NOT NULL,
    peek_id TEXT NOT NULL,
    target_server TEXT NOT NULL,
    renew_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (room_id)
);
CREATE INDEX idx_room_peeks_renew ON room_peeks(renew_at);
