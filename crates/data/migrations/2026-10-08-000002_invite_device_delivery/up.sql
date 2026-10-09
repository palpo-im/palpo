-- Global admission preserves the invitation decision; delivery is per device.
ALTER TABLE room_invite_admissions
    ADD COLUMN delivered_devices JSONB NOT NULL DEFAULT '{}'::jsonb
    CHECK (jsonb_typeof(delivered_devices) = 'object');
