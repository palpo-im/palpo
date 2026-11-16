# Pending-publication sliding sync v4

`palpo_core::client::sync_events::v4` contains independent request and response
models behind `unstable-msc4186`, adapted from
[Ruma 472a23e](https://github.com/ruma/ruma/commit/472a23ef86bc0bb40a22d856fc0af539eb9d71b7).
Ruma's MIT permission notice is retained beside the models.

The new contract places `conn_id`, `pos`, `timeout` and `set_presence` in the
JSON body. Lists use one optional inclusive range; omitting it means all
matching rooms. Required state uses include/exclude matchers and explicit lazy
membership loading. List filters preserve the difference between omitted,
true and false. Numeric ranges, limits and response counters use JavaScript-safe
unsigned integers. Call `validate()` before mutating connection state to reject
reversed ranges and more than 100 lists or room subscriptions.

Room responses distinguish full data from stripped invite/knock state. Omitted
name/avatar fields mean unchanged, while JSON null clears them. Required state
can contain deleted-state stubs; these differ from an event with empty or null
content. Extension models follow the updated typing, to-device, E2EE, account
data and receipts contracts. Existing optional profile, presence, sticky-event
and subscription extensions remain feature-gated. Unknown extensions survive
JSON round trips.

These models are the prerequisite for [#651](https://github.com/palpo-im/palpo/issues/651).
The server still serves the existing unstable endpoint with its original
models. It must implement the new connection state, filtering, state deletion,
retry and acknowledgement semantics before registering `/v4/sync` or advertising
`org.matrix.simplified_msc3575.stable`. A route alias or a lossy conversion of the
new request into the older request would silently omit client requirements.

The server integration needs HTTP regressions for initial/incremental sync,
invalid and expired positions, competing requests, subscription/range/filter
changes, state exclusions, extension enable/disable, retries and long polling.
Issue #651 remains open until those behaviors work through the new endpoint.
