-- Selective presence recipient sets (MSC4495).
--
-- Presence is only shared with a user's recipient set, and remote servers are
-- told about changes to that set as deltas rather than as a whole set on every
-- update. Producing a correct delta needs the set that was last sent, per
-- destination, and it must survive a restart: a server that forgot what it had
-- sent would either resend everything or, worse, never retract a recipient the
-- user has since denied.

-- The monotonic recipient-set stream position of each local user.
--
-- One counter per user, bumped whenever their effective recipient set changes.
-- Remote servers use it to detect that they missed an update.
CREATE TABLE presence_recipient_streams (
    user_id TEXT NOT NULL PRIMARY KEY,
    stream_id BIGINT NOT NULL
);

-- What each destination has been told about each local user's recipient set.
--
-- `recipients` holds only the destination's own users, which is what the delta
-- for that destination is computed against, and what the recovery endpoint
-- answers with. `stream_id` is the position that set corresponds to and becomes
-- the `prev_id` of the next delta.
--
-- `pending_recipients` is a delta that has been put on the wire but not yet
-- acknowledged; it is promoted into `recipients` only when the transaction
-- carrying that exact presence batch succeeds. Writing straight into
-- `recipients` instead would lose a removal whose transaction failed: the next
-- pass would see nothing left to remove while the destination still held the
-- recipient. `pending_edu_sn` ties that pending state to the selection window
-- that actually carried it, so an unrelated successful transaction cannot
-- confirm it after a restart.
CREATE TABLE presence_recipient_sets (
    user_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    stream_id BIGINT NOT NULL,
    recipients JSONB NOT NULL,
    pending_stream_id BIGINT,
    pending_recipients JSONB,
    pending_edu_sn BIGINT,
    PRIMARY KEY (user_id, server_id)
);

CREATE INDEX presence_recipient_sets_server_idx ON presence_recipient_sets (server_id);

-- Recipient sets received from remote servers, for their own users.
--
-- A remote user's presence may only be shown to the local users in their set,
-- so this is the authority for inbound filtering. `stream_id` is the position
-- the stored set corresponds to; a delta whose `prev_id` does not match it
-- means an update was missed and the set must be re-fetched.
CREATE TABLE remote_presence_recipients (
    user_id TEXT NOT NULL PRIMARY KEY,
    stream_id BIGINT NOT NULL,
    recipients JSONB NOT NULL
);
