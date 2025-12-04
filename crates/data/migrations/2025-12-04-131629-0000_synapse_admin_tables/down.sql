ALTER TABLE users DROP COLUMN IF EXISTS suspended_at;
DROP TABLE IF EXISTS user_ratelimit_override;
DROP INDEX IF EXISTS idx_user_external_ids_user_id;
DROP TABLE IF EXISTS user_external_ids;
