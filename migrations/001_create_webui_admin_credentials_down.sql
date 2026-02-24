-- Rollback migration for webui_admin_credentials table
-- This script removes the Web UI admin credentials table and its index

-- Drop the unique index
DROP INDEX IF EXISTS idx_webui_admin_single;

-- Drop the webui_admin_credentials table
DROP TABLE IF EXISTS webui_admin_credentials;
