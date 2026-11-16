# Experimental cross-room thread-update contracts

The `unstable-msc4360` core feature adds the `io.element.msc4360.threads`
request/response models and query/response types for authenticated
`GET /_matrix/client/unstable/io.element.msc4360/thread_updates`, based on
[Ruma ea7c547](https://github.com/ruma/ruma/commit/ea7c5471cf46bb04be3efeac4d9ee4c937842446)
and [MSC4360](https://github.com/matrix-org/matrix-spec-proposals/pull/4360).
It includes the pending-publication v4 protocol feature and exposes the extension
through those v4 models. It does not enable a server endpoint or discovery flag.

The extension's `enabled` flag applies independently to each request.
`include_roots` controls optional roots in sync updates. Pagination always
includes roots with bundled `m.thread` aggregation. Sync `prev_batch`, pagination
`next_batch`, and each thread's `/relations` token are separate opaque values.
An extension containing only a pagination token is not empty. Limits use safe
unsigned integers; a pagination handler must call `validate()` to reject forward
pagination and zero limits before operating on the cursor.

[#652](https://github.com/palpo-im/palpo/issues/652) remains open for server work.
The existing unstable sync endpoint continues to use its existing contracts.
Before either sync engine can expose this extension, it needs a durable stream
for reply/root changes, edits and redactions, with deterministic ordering and
bounded pagination. A reply-only lookup cannot capture root edits or aggregation
changes. The implementation must enforce joined-room and historical visibility,
redactions and ignored users on roots and bundled replies; retries, equal-position
page boundaries and membership revocation require dedicated HTTP/database tests.
Thread subscriptions remain an independent feature.
