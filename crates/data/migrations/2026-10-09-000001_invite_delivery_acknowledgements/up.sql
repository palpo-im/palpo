ALTER TABLE room_invite_admissions
    ADD COLUMN acknowledged_devices JSONB NOT NULL DEFAULT '{}';

-- Response identities are separate from the event stream. A token acknowledges
-- only the invitations included in that response, including sliding-sync ranges.
CREATE TABLE room_invite_delivery_batches (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    delivery_sn BIGINT NOT NULL,
    membership_ids BIGINT[] NOT NULL,
    batch_key BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, device_id, delivery_sn, batch_key)
);
