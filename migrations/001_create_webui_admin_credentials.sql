-- Create webui_admin_credentials table
-- This table stores Web UI admin credentials with a fixed username "admin"
-- Only one row is allowed in this table (enforced by unique index)

CREATE TABLE IF NOT EXISTS webui_admin_credentials (
    username TEXT PRIMARY KEY CHECK (username = 'admin'),
    password_hash TEXT NOT NULL,
    salt TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Create unique index to ensure only one row exists
CREATE UNIQUE INDEX IF NOT EXISTS idx_webui_admin_single 
ON webui_admin_credentials ((1));

-- Add comments for documentation
COMMENT ON TABLE webui_admin_credentials IS 
'Stores Web UI admin credentials. Only one row allowed with username=admin';

COMMENT ON COLUMN webui_admin_credentials.username IS 
'Fixed username "admin" enforced by CHECK constraint';

COMMENT ON COLUMN webui_admin_credentials.password_hash IS 
'Argon2id or bcrypt hash of the password';

COMMENT ON COLUMN webui_admin_credentials.salt IS 
'Unique salt used for password hashing';

COMMENT ON COLUMN webui_admin_credentials.created_at IS 
'Timestamp when admin account was first created';

COMMENT ON COLUMN webui_admin_credentials.updated_at IS 
'Timestamp of last password change';
