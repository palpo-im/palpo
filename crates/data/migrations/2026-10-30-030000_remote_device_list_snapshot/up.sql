-- A signed pre-membership snapshot is a proven initial version, even while
-- unsolicited EDUs for this user are still outside local tracking scope.
ALTER TABLE remote_device_list_versions
    ADD COLUMN is_initial_snapshot BOOLEAN NOT NULL DEFAULT FALSE;
