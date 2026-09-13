-- Profile change stream (MSC4262).
--
-- Sliding sync needs to answer "which profile fields changed since sync
-- position N", which the profile itself cannot answer: `user_profiles` only
-- holds current values. Every profile write appends a row here, stamped with a
-- position from the same `occur_sn_seq` sequence that orders events, so a sync
-- can select exactly the changes a client has not seen.
--
-- `occur_sn` is allocated by the same sequence default the other incremental
-- sync streams use (`user_datas`, `device_inboxes`), so the position is consumed
-- in the statement that writes the row rather than ahead of it.
--
-- `removed` distinguishes a cleared field from a field whose stored value is
-- JSON `null`; the spec allows servers to store `null` as a value, so the two
-- cases cannot be told apart from `value` alone.
CREATE TABLE user_profile_changes (
    id BIGSERIAL PRIMARY KEY,
    occur_sn BIGINT NOT NULL DEFAULT nextval('occur_sn_seq'),
    user_id TEXT NOT NULL,
    field TEXT NOT NULL,
    value JSONB,
    removed BOOLEAN NOT NULL DEFAULT FALSE
);

-- Selecting the changes past a sync position, optionally narrowed to the users
-- a syncing client shares a room with.
CREATE INDEX user_profile_changes_sn_idx ON user_profile_changes (occur_sn);
CREATE INDEX user_profile_changes_user_sn_idx ON user_profile_changes (user_id, occur_sn);

-- Remote users get a global profile row too, so that the one place sliding sync
-- reads profiles from answers for them as well.
--
-- `user_profiles_udx` cannot keep those rows unique: it is `UNIQUE (user_id,
-- room_id)` and PostgreSQL treats NULLs as distinct, so `(user, NULL)` never
-- conflicts with itself and an upsert on it silently inserts duplicates. A
-- partial index over the global rows is what actually constrains them.
DELETE FROM user_profiles a
    USING user_profiles b
    WHERE a.room_id IS NULL
      AND b.room_id IS NULL
      AND a.user_id = b.user_id
      -- Keep the newest global profile. Older Palpo versions could create more
      -- than one NULL-room row because PostgreSQL does not consider NULLs equal
      -- for a normal unique constraint.
      AND a.id < b.id;

CREATE UNIQUE INDEX user_profiles_global_udx
    ON user_profiles (user_id)
    WHERE room_id IS NULL;
