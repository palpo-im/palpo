-- MSC2753 client-side peeking: which local users are peeking which rooms.
-- A user peek implies a server-level federation peek (room_peeks) for remote
-- rooms; the last user peek leaving a room lets us drop that subscription.
CREATE TABLE user_peeks (
    id BIGSERIAL NOT NULL PRIMARY KEY,
    user_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    UNIQUE (user_id, room_id)
);
CREATE INDEX idx_user_peeks_user ON user_peeks(user_id);
CREATE INDEX idx_user_peeks_room ON user_peeks(room_id);
