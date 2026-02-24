-- Web UI Admin Credentials table for database-backed authentication
-- This table stores credentials for the Web UI admin (Tier 1) which operates
-- independently of the Palpo Matrix server. The username is fixed as 'admin'
-- and only one credential record can exist.

CREATE TABLE webui_admin_credentials (
    -- Username is fixed as 'admin' enforced by CHECK constraint
    username TEXT PRIMARY KEY CHECK (username = 'admin'),
    
    -- Password hash using Argon2 or bcrypt
    password_hash TEXT NOT NULL,
    
    -- Salt used for password hashing
    salt TEXT NOT NULL,
    
    -- Timestamp when the credential was created
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    
    -- Timestamp when the credential was last updated
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Unique index to ensure only one row can exist in the table
-- This uses a partial unique index on a constant value to enforce single-row constraint
CREATE UNIQUE INDEX idx_webui_admin_single_row ON webui_admin_credentials ((1));

-- Comment on table
COMMENT ON TABLE webui_admin_credentials IS 'Stores Web UI admin credentials for database-backed authentication (Tier 1). Only one admin account with username "admin" can exist.';

-- Comment on columns
COMMENT ON COLUMN webui_admin_credentials.username IS 'Fixed username "admin" enforced by CHECK constraint';
COMMENT ON COLUMN webui_admin_credentials.password_hash IS 'Argon2 or bcrypt password hash';
COMMENT ON COLUMN webui_admin_credentials.salt IS 'Salt used for password hashing';
COMMENT ON COLUMN webui_admin_credentials.created_at IS 'Timestamp when credential was created';
COMMENT ON COLUMN webui_admin_credentials.updated_at IS 'Timestamp when credential was last updated';
