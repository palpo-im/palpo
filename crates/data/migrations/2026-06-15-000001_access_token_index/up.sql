-- Access token authentication hot path.
--
-- Every authenticated client/federation-puppet request looks up
-- `user_access_tokens` by `WHERE token = ?` (see crates/server/src/hoops/auth.rs).
-- The table previously only had a UNIQUE (user_id, device_id) constraint
-- (named user_access_tokens_token_udx, despite covering user_id+device_id), so
-- the token lookup degraded to a sequential scan that grows with the number of
-- active sessions. Tokens are 32-char random strings and are globally unique, so
-- a UNIQUE index is both correct (guards against accidental collisions) and gives
-- the planner an index-only equality lookup on the auth hot path.
CREATE UNIQUE INDEX IF NOT EXISTS user_access_tokens_token_only_udx
    ON user_access_tokens USING btree (token);
