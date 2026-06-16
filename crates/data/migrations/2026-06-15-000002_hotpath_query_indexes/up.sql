-- Hot-path query indexes for small/medium tables.
--
-- Each index below covers a `WHERE` pattern that is exercised on a frequently
-- hit path but had no usable index (the existing UNIQUE constraints don't have
-- the filtered column as their left-most prefix). Verified against the actual
-- query sites cited in each comment. The large per-event tables (events,
-- event_points) are handled separately because building an index on them needs
-- a non-transactional CONCURRENTLY migration.

-- get_devices: `WHERE user_id = ?` (crates/data/src/user/device.rs).
-- UNIQUE (device_id, user_id) can't serve a user_id-only lookup.
CREATE INDEX IF NOT EXISTS user_devices_user_id_idx
    ON user_devices USING btree (user_id);

-- get_password_hash: `WHERE user_id = ? ORDER BY id DESC` (crates/data/src/user/password.rs).
CREATE INDEX IF NOT EXISTS user_passwords_user_id_idx
    ON user_passwords USING btree (user_id, id DESC);

-- get_pushers / get_push_keys / delete_*: `WHERE user_id = ?`
-- (crates/data/src/user/pusher.rs). Existing index is only (app_id, pushkey).
CREATE INDEX IF NOT EXISTS user_pushers_user_id_idx
    ON user_pushers USING btree (user_id);

-- get_user_by_threepid: `WHERE medium = ? AND address = ?`
-- (crates/data/src/user.rs) on the 3pid login path.
CREATE INDEX IF NOT EXISTS user_threepids_medium_address_idx
    ON user_threepids USING btree (medium, address);

-- relation aggregation / thread fetch: `WHERE event_id = ?` (single and IN)
-- (crates/data/src/room.rs, crates/server/src/room/timeline.rs).
-- UNIQUE (room_id, event_id, child_id, rel_type) can't serve an event_id-only lookup.
CREATE INDEX IF NOT EXISTS event_relations_event_id_idx
    ON event_relations USING btree (event_id);

-- federation send queue dispatch: `WHERE state = 'pending'` and
-- `WHERE kind = ? AND state = 'pending' AND ...` (crates/server/src/sending.rs).
-- state left-most also serves the state-only dispatcher poll.
CREATE INDEX IF NOT EXISTS outgoing_requests_state_kind_idx
    ON outgoing_requests USING btree (state, kind);
