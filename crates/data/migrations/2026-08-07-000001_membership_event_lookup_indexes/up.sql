-- Later-join history-visibility checks first locate accepted membership events in a
-- room, then walk backwards through event_edges. Keep both the exact-user and
-- server-wide candidate lookups off the full events table scan.
CREATE INDEX CONCURRENTLY IF NOT EXISTS events_membership_user_depth_idx
    ON events USING btree (room_id, state_key, depth, id)
    WHERE ty = 'm.room.member'
      AND is_outlier = FALSE
      AND soft_failed = FALSE
      AND is_rejected = FALSE;

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
