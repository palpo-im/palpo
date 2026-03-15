-- Rollback Web UI Admin Credentials table
-- This removes the webui_admin_credentials table and its associated index

-- Drop the unique index
DROP INDEX IF EXISTS idx_webui_admin_single_row;

-- Drop the table
DROP TABLE IF EXISTS webui_admin_credentials;
