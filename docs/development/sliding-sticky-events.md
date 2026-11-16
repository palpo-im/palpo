# Sticky events in simplified sliding sync

Build with `--features unstable-msc4480` to opt in to MSC4480 on the existing
`POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync` endpoint.
This does not add the separate stable `/v4/sync` contract tracked in #651.

```json
{"conn_id":"rtc","extensions":{"org.matrix.msc4354.sticky_events":{
  "enabled":true,"limit":100,"lists":["rooms"],"rooms":["*"]
}}}
```

The extension must be enabled on every request. `limit` defaults to 100 and is
capped at 100 per room; zero is invalid. Omitted `lists` selects all request
lists; an empty array selects none. Omitted `rooms` selects all global room
subscriptions, `*` selects those subscriptions, and explicit room IDs add those
rooms independently of the top-level room response. Only joined rooms qualify.

Responses use `extensions.org.matrix.msc4354.sticky_events.rooms[room_id].events`.
Events already in the timeline are not repeated. Both copies carry the existing
MSC4354 unstable `unsigned.msc4354_sticky_duration_ttl_ms` field, calculated from
the original persisted expiry. Lazy member state includes sticky-event senders.
Ignored, expired and redacted events are excluded, including when replaying a page.

The complete opaque `pos` acknowledges the offered page; there is no separate
extension cursor. Pages contain the oldest unacknowledged events, and queued
events drain on following requests without new room activity. Connection state is
persisted in the existing versioned sliding-sync row, including offered event IDs
and per-room progress. Restart reloads the outstanding page. The offer history is
bounded to 16 responses; evicting an unacknowledged offer causes safe redelivery.
Configuration races and failed persistence return an error instead of advancing
past unsaved delivery. Disabling the extension, scope exit or a new join period
resets the affected room snapshot. Expiry and permission changes are rechecked.

New or re-entering rooms send all still-sticky events, including events predating
the current join as required by MSC4354. Classic sync behavior stays separate.
