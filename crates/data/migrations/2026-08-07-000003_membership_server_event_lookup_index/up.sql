-- Server-wide later-join checks use the homeserver portion of membership state
-- keys. Keep this concurrent DDL in its own batch so PostgreSQL does not create
-- an implicit transaction block around it.
CREATE INDEX CONCURRENTLY IF NOT EXISTS events_membership_server_depth_idx
    ON events USING btree (
        room_id,
        (substring(state_key from position(':' in state_key) + 1)),
        depth,
        id
    )
    WHERE ty = 'm.room.member'
      AND is_outlier = FALSE
      AND soft_failed = FALSE
      AND is_rejected = FALSE;
