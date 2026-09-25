-- Optimistic concurrency for the sliding sync connection cache.
--
-- Every Palpo instance keeps an in-memory copy of each connection's cache and
-- writes the whole blob back after changing it. Without a version, an instance
-- holding a stale copy silently overwrote a newer row written by another
-- instance, and never noticed that its own copy was out of date.
--
-- Versions come from a dedicated sequence rather than a per-row counter, so a
-- connection that is deleted and recreated (a fresh `pos`-less sync) never
-- reuses a version another instance may still hold in memory.
CREATE SEQUENCE sliding_sync_connection_version_seq;

ALTER TABLE sliding_sync_connections
    ADD COLUMN version BIGINT NOT NULL DEFAULT 0;
