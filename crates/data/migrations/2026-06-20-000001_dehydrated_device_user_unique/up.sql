-- Drop key material that belonged to dehydrated devices pruned below, but keep
-- anything still referenced by the surviving (highest-id) row per user so a row
-- that merely re-used the same device_id does not lose its keys.
WITH survivor AS (
    SELECT DISTINCT ON (user_id) user_id, device_id
    FROM user_dehydrated_devices
    ORDER BY user_id, id DESC
),
pruned AS (
    SELECT DISTINCT d.user_id, d.device_id
    FROM user_dehydrated_devices d
    WHERE NOT EXISTS (
        SELECT 1 FROM survivor s
        WHERE s.user_id = d.user_id AND s.device_id = d.device_id
    )
)
DELETE FROM e2e_device_keys k
USING pruned p
WHERE k.user_id = p.user_id AND k.device_id = p.device_id;

WITH survivor AS (
    SELECT DISTINCT ON (user_id) user_id, device_id
    FROM user_dehydrated_devices
    ORDER BY user_id, id DESC
),
pruned AS (
    SELECT DISTINCT d.user_id, d.device_id
    FROM user_dehydrated_devices d
    WHERE NOT EXISTS (
        SELECT 1 FROM survivor s
        WHERE s.user_id = d.user_id AND s.device_id = d.device_id
    )
)
DELETE FROM e2e_one_time_keys k
USING pruned p
WHERE k.user_id = p.user_id AND k.device_id = p.device_id;

WITH survivor AS (
    SELECT DISTINCT ON (user_id) user_id, device_id
    FROM user_dehydrated_devices
    ORDER BY user_id, id DESC
),
pruned AS (
    SELECT DISTINCT d.user_id, d.device_id
    FROM user_dehydrated_devices d
    WHERE NOT EXISTS (
        SELECT 1 FROM survivor s
        WHERE s.user_id = d.user_id AND s.device_id = d.device_id
    )
)
DELETE FROM e2e_fallback_keys k
USING pruned p
WHERE k.user_id = p.user_id AND k.device_id = p.device_id;

WITH survivor AS (
    SELECT DISTINCT ON (user_id) user_id, device_id
    FROM user_dehydrated_devices
    ORDER BY user_id, id DESC
),
pruned AS (
    SELECT DISTINCT d.user_id, d.device_id
    FROM user_dehydrated_devices d
    WHERE NOT EXISTS (
        SELECT 1 FROM survivor s
        WHERE s.user_id = d.user_id AND s.device_id = d.device_id
    )
)
DELETE FROM e2e_cross_signing_sigs s
USING pruned p
WHERE s.target_user_id = p.user_id AND s.target_device_id = p.device_id;

DELETE FROM user_dehydrated_devices a
USING user_dehydrated_devices b
WHERE a.user_id = b.user_id
  AND a.id < b.id;

CREATE UNIQUE INDEX IF NOT EXISTS user_dehydrated_devices_user_unique_idx
    ON user_dehydrated_devices USING btree
    (user_id ASC NULLS LAST);
