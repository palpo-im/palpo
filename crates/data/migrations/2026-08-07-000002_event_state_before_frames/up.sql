-- `frame_id` is mutable for state events because sync-delta bookkeeping advances
-- current state events to newer frames. History visibility needs the state at the
-- event, so keep a separate immutable frame recorded when the event is appended.
-- Existing state events remain NULL and are handled fail-closed by the server;
-- copying their mutable frame would recreate the disclosure this column prevents.
ALTER TABLE event_points
    ADD COLUMN IF NOT EXISTS before_frame_id bigint;
