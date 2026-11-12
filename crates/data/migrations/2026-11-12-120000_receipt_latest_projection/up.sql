-- The log remains append-only: outgoing consumer cursors retain their history.
CREATE INDEX event_receipts_latest_key ON event_receipts
    (room_id, user_id, ty, (COALESCE(json_data->>'thread_id', '')), sn DESC);
