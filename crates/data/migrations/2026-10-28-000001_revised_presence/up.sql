CREATE TABLE revised_presence_clients (
    user_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at BIGINT,
    PRIMARY KEY (user_id, client_id)
);
CREATE INDEX revised_presence_clients_expiry ON revised_presence_clients (expires_at);
ALTER TABLE user_presences ADD COLUMN revised_status JSONB NOT NULL DEFAULT '{}';
UPDATE user_presences SET revised_status=jsonb_build_object('msg',status_msg) WHERE status_msg IS NOT NULL;
CREATE TABLE revised_presence_outages (
    destination TEXT PRIMARY KEY,
    failed_at BIGINT NOT NULL,
    notified BOOLEAN NOT NULL DEFAULT FALSE
);
