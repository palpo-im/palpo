ALTER TABLE user_access_tokens DROP COLUMN IF EXISTS oauth_client_id;
DROP TABLE IF EXISTS oauth_authorization_codes;
DROP TABLE IF EXISTS oauth_clients;
