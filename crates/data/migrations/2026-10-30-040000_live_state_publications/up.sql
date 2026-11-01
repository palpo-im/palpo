-- Event identity/stream ordering can predate live timeline acceptance. Keep
-- publication order and its immutable state snapshot separately.
ALTER TABLE event_points
    ADD COLUMN live_published_sn BIGINT,
    ADD COLUMN live_published_frame_id BIGINT,
    ADD CONSTRAINT event_points_live_publication_pair CHECK
        ((live_published_sn IS NULL) = (live_published_frame_id IS NULL));

-- Only already accepted, non-backfill events have a legacy live publication.
-- Cached outliers and rejected/soft-failed events must remain unpublished.
UPDATE event_points AS point
SET live_published_sn = event.stream_ordering,
    live_published_frame_id = point.frame_id
FROM events AS event
WHERE event.id = point.event_id AND event.room_id = point.room_id
    AND point.frame_id IS NOT NULL
    AND NOT event.is_outlier AND NOT event.soft_failed AND NOT event.is_rejected
    AND event.stream_ordering >= 0;

CREATE INDEX event_points_room_live_publication
    ON event_points (room_id, live_published_sn DESC)
    WHERE live_published_sn IS NOT NULL;
