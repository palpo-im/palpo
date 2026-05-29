-- MSC2753 client-side peeking: which local user *devices* are peeking which
-- rooms. Peeks are per-device (a client decides what to peek), so one device's
-- peek/unpeek must not affect another device of the same account. A user peek
-- implies a server-level federation peek (room_peeks) for remote rooms; the
-- last user-device peek leaving a room lets us drop that subscription.
CREATE TABLE user_peeks (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (user_id, device_id, room_id)
);
CREATE INDEX idx_user_peeks_user ON user_peeks(user_id);
CREATE INDEX idx_user_peeks_user_device ON user_peeks(user_id, device_id);
CREATE INDEX idx_user_peeks_room ON user_peeks(room_id);
