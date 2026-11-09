-- Latest-per-room markers cannot answer a bounded historical /keys/changes
-- request after another update overwrites them. Keep the user/position journal
-- transactionally, including writes by older application instances.
CREATE TABLE e2e_key_change_history (
    user_id TEXT NOT NULL,
    occur_sn BIGINT NOT NULL,
    PRIMARY KEY (user_id, occur_sn)
);

-- Cover the handoff from backfill to trigger without a concurrent writer gap.
LOCK TABLE e2e_key_changes IN SHARE ROW EXCLUSIVE MODE;

INSERT INTO e2e_key_change_history (user_id, occur_sn)
SELECT DISTINCT user_id, occur_sn FROM e2e_key_changes;

CREATE FUNCTION record_device_key_change_history() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO e2e_key_change_history (user_id, occur_sn)
    VALUES (NEW.user_id, NEW.occur_sn)
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE TRIGGER record_device_key_change_history
-- BEFORE INSERT also observes an older attempted notification whose
-- ON CONFLICT DO UPDATE WHERE clause deliberately retains a newer marker.
-- An upsert can invoke INSERT and UPDATE triggers for the same position; the
-- journal primary key deduplicates them within the writer's transaction.
BEFORE INSERT OR UPDATE ON e2e_key_changes
FOR EACH ROW EXECUTE FUNCTION record_device_key_change_history();
