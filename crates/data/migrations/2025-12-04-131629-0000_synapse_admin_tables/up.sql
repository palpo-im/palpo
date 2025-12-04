CREATE TABLE user_external_ids (
    id BIGSERIAL PRIMARY KEY,
    auth_provider TEXT NOT NULL,
    external_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    UNIQUE(auth_provider, external_id)
);

CREATE INDEX idx_user_external_ids_user_id ON user_external_ids(user_id);

CREATE TABLE user_ratelimit_override (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    messages_per_second INTEGER,
    burst_count INTEGER
);

ALTER TABLE users ADD COLUMN suspended_at BIGINT;
