DROP INDEX event_points_room_live_publication;
ALTER TABLE event_points
    DROP CONSTRAINT event_points_live_publication_pair,
    DROP COLUMN live_published_frame_id,
    DROP COLUMN live_published_sn;
