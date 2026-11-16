# Administration and delivery contracts

The vendor endpoints below are available under both `/_palpo/admin` and
`/_synapse/admin`. They require the existing administrator authentication and
permission checks; they are not new Matrix Client-Server API endpoints.

`GET /v1/capabilities` returns `{ "capabilities": { "feature": true } }`.
Clients must feature-detect additions rather than infer admin features from the
standard Matrix `/versions` response. Current additions are:

| Feature | Contract |
| --- | --- |
| `room_aliases` | `GET /v1/rooms/{room_id}/aliases` returns `{aliases:[...]}` containing local directory mappings, independently of canonical/preferred aliases and administrator membership. |
| `alias_delete_preserves_state` | Deleting one local alias only removes that alias from preferred state, preserving unrelated canonical/alternative aliases and custom content. Directory permissions remain independent of room-state permissions. |
| `room_members_pagination` | `GET /v1/rooms/{room_id}/members?from=0&limit=50` returns `{members:[...],total:n}`. `from` must be non-negative and `limit` is 1–1000. Ordering is deterministic. Omitting the limit retains the legacy unpaginated response. |
| `make_room_admin` | `POST /v1/rooms/{room_id}/make_room_admin` with `{user_id:...}` promotes an already joined member. It uses a joined local actor with real permission to update power levels, preserves other state and runs normal event authorization. No suitable local actor gives 403; it cannot invent a remote signature or override federation authorization. Existing privileged creators succeed without adding an invalid users entry. |
| `report_event_details` | Report detail and status-update responses obtain `sender` and `event_json` from the actual reported event. The report's `user_id` is the reporter, not the sender. Missing/purged events omit sender/event_json instead of pretending that reason/score is an event. |
| `user_suspended` | User detail/list responses contain the persisted `suspended` boolean. Change it through `PUT /v1/suspend/{user_id}` with `{suspend:bool}`, not the general user PUT. |
| `destination_search` | Destination list filtering precedes total/count and pagination; `destination` is a case-insensitive substring, `dir=f/b` controls deterministic ordering. Absent retry telemetry retains its existing null/zero values, with destination as the stable tie-breaker. |
| `media_keep_profiles` | `POST /v1/media/delete?before_ts=...&size_gt=...&keep_profiles=true` protects retained user/member avatar references and room-avatar events, including historical room avatars. The default is true; false explicitly permits deleting those files. |
| `bulk_device_deletion` | `POST /v2/users/{user_id}/delete_devices` with `{devices:[...]}` atomically revokes selected local Matrix devices and persists notifications. Failures propagate. This does not revoke a separate OAuth provider's grants. |
| `appservice_url_cas` | Existing `PUT /v1/appservices/{id}/url` changes only the callback URL and requires the exact `expected_url`, including explicit null. There is no full registration replacement API; clients must not label this operation as token/namespace editing. |

Classic sync limits to-device delivery to a preferred batch of 100 events and
512 KiB of event JSON (using the stored text length as a conservative budget).
The database computes the contiguous delivery boundary without first loading a
full batch of pending JSON payloads into the application. A single larger event is still delivered so
that the queue can make progress. The entire shared stream window is capped at
the last included inbox position: the next since token cannot acknowledge
messages omitted from this batch. Consumers must allow a single larger message
or provide an explicit oversized-message recovery path; they must not drop it
and confirm the rest of the batch.

The [Matrix event size limit](https://spec.matrix.org/v1.12/client-server-api/#size-limits)
is 65,536 UTF-8 bytes for the complete canonical federation event, including
signatures. Hash-input serialization is not a substitute for this check. Local
publication validates again after additional signatures and before persistence;
internal client/storage-only metadata is not part of the federation format.
