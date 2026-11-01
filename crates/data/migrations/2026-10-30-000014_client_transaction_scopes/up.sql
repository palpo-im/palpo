ALTER TABLE event_idempotents ADD COLUMN endpoint text NOT NULL DEFAULT '';

-- Old room-event rows contain enough information to reconstruct their scope.
-- Old to-device rows did not record the event type; retain those as legacy rows
-- rather than letting ambiguous records suppress a different endpoint's send.
UPDATE event_idempotents i SET endpoint = 'room/send/' || e.ty
FROM events e WHERE i.event_id = e.id AND i.room_id IS NOT NULL;

DELETE FROM event_idempotents WHERE id IN (
    SELECT id FROM (
        SELECT id, row_number() OVER (
            PARTITION BY user_id, device_id, room_id, endpoint, txn_id
            ORDER BY id DESC
        ) AS ordinal FROM event_idempotents
    ) duplicates WHERE ordinal > 1
);
DROP INDEX event_idempotents_txn_id_udx;
CREATE UNIQUE INDEX event_idempotents_txn_id_udx ON event_idempotents
    (user_id, device_id, room_id, endpoint, txn_id) NULLS NOT DISTINCT;
