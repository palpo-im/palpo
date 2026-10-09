-- Permanent content-expiry markers retain the event graph and prevent backfill
-- or a later, longer policy from resurrecting purged content.
CREATE TABLE event_retention_expired (
    event_id TEXT PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
    expired_at BIGINT NOT NULL
);

CREATE FUNCTION palpo_retention_scrub(value JSON) RETURNS JSON
LANGUAGE SQL IMMUTABLE AS $$
    SELECT (COALESCE(jsonb_object_agg(key, val), '{}'::jsonb)
        || jsonb_build_object('content', CASE
            WHEN value->>'type' = 'm.room.redaction' AND value->'content'->'redacts' IS NOT NULL
                THEN jsonb_build_object('redacts', value->'content'->'redacts')
            ELSE '{}'::jsonb END))::json
    FROM jsonb_each(value::jsonb) AS fields(key, val)
    WHERE key = ANY(ARRAY['event_id','room_id','type','sender','origin','origin_server_ts',
        'depth','prev_events','prev_state','auth_events','hashes','signatures','redacts']);
$$;

CREATE FUNCTION palpo_retention_guard() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    -- Serialize against the expiry transaction and all other writers to this event.
    PERFORM 1 FROM events WHERE id = NEW.event_id FOR UPDATE;
    IF EXISTS (SELECT 1 FROM event_retention_expired WHERE event_id = NEW.event_id) THEN
        NEW.json_data := palpo_retention_scrub(NEW.json_data);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER event_retention_content_guard BEFORE INSERT OR UPDATE OF json_data
ON event_datas FOR EACH ROW EXECUTE FUNCTION palpo_retention_guard();

CREATE FUNCTION palpo_retention_index_guard() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM events WHERE id = NEW.event_id FOR UPDATE;
    IF EXISTS (SELECT 1 FROM event_retention_expired WHERE event_id = NEW.event_id) THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER event_retention_search_guard BEFORE INSERT OR UPDATE ON event_searches
FOR EACH ROW EXECUTE FUNCTION palpo_retention_index_guard();

-- Finalized delayed-send records can retain another copy of the event content.
-- A sender finishing after expiry must not put that copy back into storage.
CREATE FUNCTION palpo_retention_delayed_guard() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.event_id IS NOT NULL THEN
        PERFORM 1 FROM events WHERE id = NEW.event_id FOR UPDATE;
        IF EXISTS (SELECT 1 FROM event_retention_expired WHERE event_id = NEW.event_id) THEN
            NEW.content := '{}'::jsonb;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER event_retention_delayed_guard BEFORE INSERT OR UPDATE OF event_id, content
ON delayed_events FOR EACH ROW EXECUTE FUNCTION palpo_retention_delayed_guard();
