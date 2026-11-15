# Batch profile queries (MSC4536)

Set `enable_batch_profile_queries = true` to expose authenticated, rate-limited
`POST /_matrix/client/unstable/org.matrix.msc4536/profile/query`. It is disabled by
default, and `/versions` advertises `org.matrix.msc4536` only when enabled.

```json
{"users":["@alice:example.com"],"fields":["displayname","org.example.status"]}
```

The response is a user-ID-to-profile object without an envelope. Omit `fields` to
return all standard and custom fields. An explicit empty field list is invalid.
Requests accept 1–100 IDs before deduplication and 1–30 field names, each 1–255
UTF-8 bytes. Missing `users` returns `M_MISSING_PARAM`; invalid values return
`M_BAD_JSON`. The Matrix request limit applies, and responses are capped at 8 MiB.
POST requests share the existing per-user `rc_message` rate limiter.

Following the current MSC4536 proposal, the batch endpoint never fetches profiles
over federation. It returns known active local global profiles in one database
query. Remote users, unknown or deactivated users, room-only profiles and users
with no known fields are omitted. If a known profile has none of the requested
fields, its selected profile is an empty object. Unset standard fields are omitted.
Custom values retain their JSON types. Profile reads require authentication even
when public single-profile reads are enabled; this endpoint adds no visibility
permissions. Single-user profile lookup behavior remains unchanged.

Remote push/cache support is a separate integration; a two-homeserver test for this
endpoint must verify that batch queries cause no remote fetch, rather than expect
the endpoint to perform a federation lookup.
