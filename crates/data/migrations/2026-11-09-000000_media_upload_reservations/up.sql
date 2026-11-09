-- New asynchronous uploads must be reserved by their authenticated creator.
-- Existing media remains readable; untracked, unused pre-upgrade MXCs must be recreated.
CREATE TABLE media_upload_reservations (
    origin_server TEXT NOT NULL,
    media_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    expires_at BIGINT NOT NULL,
    uploaded BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (origin_server, media_id)
);
