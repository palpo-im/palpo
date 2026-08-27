CREATE INDEX CONCURRENTLY IF NOT EXISTS outgoing_requests_user_id_idx
    ON outgoing_requests USING btree (user_id);
