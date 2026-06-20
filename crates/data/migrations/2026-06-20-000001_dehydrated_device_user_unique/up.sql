DELETE FROM user_dehydrated_devices a
USING user_dehydrated_devices b
WHERE a.user_id = b.user_id
  AND a.id < b.id;

CREATE UNIQUE INDEX IF NOT EXISTS user_dehydrated_devices_user_unique_idx
    ON user_dehydrated_devices USING btree
    (user_id ASC NULLS LAST);
