DROP INDEX event_idempotents_txn_id_udx;
ALTER TABLE event_idempotents DROP COLUMN endpoint;
DELETE FROM event_idempotents WHERE id IN (
    SELECT id FROM (
        SELECT id, row_number() OVER (
            PARTITION BY user_id, device_id, room_id, txn_id ORDER BY id DESC
        ) AS ordinal FROM event_idempotents
    ) duplicates WHERE ordinal > 1
);
CREATE UNIQUE INDEX event_idempotents_txn_id_udx ON event_idempotents
    (txn_id, room_id, user_id, device_id);
