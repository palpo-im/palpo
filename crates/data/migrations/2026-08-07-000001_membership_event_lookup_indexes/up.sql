-- Later-join history-visibility checks first locate accepted membership events in a
-- room, then walk backwards through event_edges. Keep the exact-user lookup off the
-- full events table scan. Concurrent index DDL must be the only statement in this
-- non-transactional migration because PostgreSQL wraps multi-statement batches in
-- an implicit transaction block.
CREATE INDEX CONCURRENTLY IF NOT EXISTS events_membership_user_depth_idx
    ON events USING btree (room_id, state_key, depth, id)
    WHERE ty = 'm.room.member'
      AND is_outlier = FALSE
      AND soft_failed = FALSE
      AND is_rejected = FALSE;
