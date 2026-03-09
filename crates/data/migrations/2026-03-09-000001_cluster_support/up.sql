-- Cluster support migration
-- Moves in-memory state to database for multi-instance deployment

-- 1. Typing state table (replaces in-memory TYPING / LAST_TYPING_UPDATE)
CREATE TABLE room_typings (
    room_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    timeout_at BIGINT NOT NULL,
    occur_sn BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (room_id, user_id)
);

-- 2. Sliding sync connection cache (replaces in-memory CONNECTIONS)
CREATE TABLE sliding_sync_connections (
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    conn_id TEXT NOT NULL DEFAULT '',
    cache_data JSONB NOT NULL DEFAULT '{}',
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, device_id, conn_id)
);

-- 3. Add request_body column to user_uiaa_datas (replaces in-memory UIAA_REQUESTS)
ALTER TABLE user_uiaa_datas ADD COLUMN request_body JSONB;

-- 4. Add retry state columns to outgoing_requests (replaces in-memory TransactionStatus)
ALTER TABLE outgoing_requests ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE outgoing_requests ADD COLUMN last_failed_at BIGINT;
