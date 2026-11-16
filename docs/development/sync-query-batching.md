# Classic sync query batching

Classic sync captures the current and previous publication frames for joined rooms
in SQL batches of at most 256 room IDs. The same publication, rejection, outlier and
soft-failure predicates as the scalar lookup apply. Work uses the captured response
window even if another event is published while the response is being built.

An event section with `limit: 0`, `types: []` or `not_types: ["*"]` cannot return
events. Ephemeral and room account-data payload queries are omitted for those
sections. Timeline type filters that cannot match skip event reads while retaining
publication-gap metadata. A timeline zero limit alone still probes one visible
event to preserve its truncation flag. Partial type filters run normally. Membership state,
one-time keys, sticky events, notification counts and independently enabled
streams continue to be evaluated; a filtered room is not assumed to be inactive.

Notification summary rows are loaded only when the existing response rules emit
room counts or the request enables per-thread counts. Unchanged, event-filtered
rooms still check read progress independently, but avoid reading every thread's
unused summary. The SQL regression checks this across 12 rooms, and separately
verifies thread-only delivery, aggregate counts and required-query failures.

The isolated SQL regression compares scalar and batch frame results over 12 rooms,
requires fewer queries for the batch, and checks one current/previous batch per
sync instead of per-room frame reads. It also verifies independent typing,
captured-frame stability after later writes, and database error propagation.

This reduces the query multipliers described in #650. It does not claim that all
unchanged joined rooms can be skipped: that requires a complete change index for
receipts, typing, account data, read state, membership and sticky eligibility as
well as timeline publications. #650 remains open for that work and deployment
latency measurement.
