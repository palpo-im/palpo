-- Federation stream IDs are opaque per-user versions, not local sequence numbers.
-- TEXT preserves the entire unsigned range without imposing a numeric ordering.
CREATE TABLE remote_device_list_versions (
    user_id TEXT NOT NULL,
    stream_id TEXT NOT NULL,
    received_order BIGSERIAL NOT NULL,
    PRIMARY KEY (user_id, stream_id)
);
CREATE INDEX remote_device_list_versions_receive_order
    ON remote_device_list_versions (user_id, received_order DESC);
