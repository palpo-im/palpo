ALTER TABLE user_profiles ADD COLUMN fields JSONB NOT NULL DEFAULT '{}'::jsonb;
