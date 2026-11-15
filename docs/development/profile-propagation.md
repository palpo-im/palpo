# Profile mutation propagation

Client profile writes, administrator user updates and MAS profile operations use
the shared service in `crates/server/src/user/profile.rs`. Changing a display name
updates only that field of each current room membership. Avatar writes update
the avatar and blurhash together; administrator and MAS avatar writes clear the
previous blurhash. Other membership fields remain intact.

The global profile transaction commits first. The service then processes joined
rooms in a stable order. It takes each room's state lock before reading current
membership and the latest global profile, skips users who have left, and appends
a member event only when the selected fields differ. Presence is refreshed after
all room updates complete. A successful response certifies completion of these
steps, including federation queueing through the normal member-event delivery
path. This certifies queueing, not acknowledgement by a remote server.

A failed request can leave the global value and some room updates committed.
The caller must retry the same requested value: propagation runs even when the
global value already matches. Matching rooms reuse the current member event and
requeue its PDU, repairing failures after state publication without creating a
new member event. This may enqueue the same event ID more than once; federation
peers can deduplicate that ID. Remaining rooms receive their new member events.
No global rollback or automatic background retry is implied.

Administrator JSON fields distinguish omission from explicit `null`: omission
keeps the value, while `null` or an empty string clears it. MAS supports its
explicit unset fields and endpoints. Clients use the existing DELETE routes.

The isolated PostgreSQL regression is
`user::profile::tests::review_profile_writes_propagate_across_routes_and_retry_partial_rooms`.
It uses real client/admin/MAS routes, checks room state, incremental sync,
presence publication and federation queue entries, and injects both a second-room
append failure and a post-publication federation enqueue failure before retrying
the identical request. It requires an empty
dedicated `PALPO_TEST_DATABASE_URL` and must run alone.
