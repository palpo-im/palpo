-- Paginating a device's to-device inbox by stream position (MSC3814).
--
-- `device_inboxes_user_device_idx` covers only `(user_id, device_id)`, so
-- reading a rehydrating device's inbox in batches made PostgreSQL scan and
-- sort the whole remaining inbox for every page -- work that grows with the
-- square of the inbox size over a full pagination.
CREATE INDEX device_inboxes_user_device_sn_idx
    ON device_inboxes (user_id, device_id, occur_sn);
