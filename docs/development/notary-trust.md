# Server key and notary trust boundaries

Key retrieval uses the configured destination's HTTPS endpoint as its identity
anchor. An origin key document must name that origin and have a valid signature
from one of its advertised current keys. Its self-signature proves possession of
the corresponding private key; it does not independently prove a server name.
TLS certificate validation and federation destination resolution remain essential.

Batch notary requests go to the configured notary, with queried server names only
in the request body. GET and POST responses are restricted to the requested
servers before any document is cached. Per the Matrix API, a response may contain
additional key IDs for a requested server; documents are not edited to remove
those keys, and callers select the specific requested ID.

Each nonempty, scoped notary response makes one additional direct HTTPS
`/_matrix/key/v2/server` request to that configured notary. This bootstrap must
have the expected server name, a valid current self-signature and an unexpired
validity bound. Generic cached keys and keys self-reported in the queried
document cannot bootstrap a notary. This direct request is to the trusted notary,
including in `only_query_trusted_key_servers` mode, rather than to an arbitrary
queried origin. Every accepted returned document must then have both a valid
origin signature and that notary's attestation. Invalid documents in a mixed
response are discarded individually. Unknown signed extension fields survive
validation, caching and notary serialization.

The additional request adds network latency and consumes the existing federation
permit budget. There is deliberately no unproven bootstrap cache shortcut or
new pinning configuration. Federation deadlines and destination/URL restrictions
remain in force. Deployments which deliberately disable TLS verification also
weaken the identity anchor described here.

## Upgrade and historical documents

Migration `2026-11-10-030000_verified_server_key_documents` creates a separate
multi-document cache. It records each signed JSON body, its canonical SHA-256,
validation policy version, direct/notary source, source server and verification
time. Signed bodies are never merged or overwritten; duplicate bodies update
their last verified observation metadata. Reads check version, digest, identity,
origin signature and whether the recorded source is still trusted.

Existing `server_signing_keys` rows remain intact for compatibility and historical
inspection, but are **not promoted** into trusted documents. New HTTP/event key
selection and prefetch availability use only the verified cache (plus the active
configured local key). After upgrade, remote documents must be obtained again
through a trusted path. If an old key exists only in the unproven legacy cache
and cannot be retrieved from its origin or a trusted notary, validation fails
closed; copying those rows into the new table is not an upgrade procedure.

HTTP authentication uses the latest verified document only, requires its lifetime
to cover the current time, and never accepts an explicitly retired key. Event
validation can use multiple verified historical documents, preserving each
document's original validity and the seven-day bound anchored to its verification
time. A verified explicit retirement applies across all saved documents; replaying
an older current-key document cannot undo it. Legacy room versions continue to
ignore key expiry according to their room-version rules. The active configured
local key remains the authoritative current local key.

This cache retains historical documents and does not install a pruning policy;
capacity and query cost should be measured before defining one that preserves
historical verification. The legacy merge helper still recomposes its legacy
index and signatures; that index is excluded from authentication and notary
publication. Removing those obsolete APIs and designing archival/pruning policy
are separate follow-up work. No automatic import or destruction of old data is
included. Rollback drops the new cache table, retains legacy rows, and requires
fresh verification when upgrading again.

## Regression coverage and evidence

The dedicated PostgreSQL regression owns two independent loopback TLS peers and
uses the actual federation request transport and HTTP authentication hoop:

```text
cargo test --locked -p palpo --lib server_key::notary_tests::review_notary_routes_verify_attestations_and_separate_request_from_event_keys -- --ignored --exact --test-threads=1 --nocapture
```

Run it alone with an empty dedicated `PALPO_TEST_DATABASE_URL`. The CI exact-case
manifest runs it in fresh processes/databases with default and all features. It
asserts actual POST routing, direct notary bootstrap, origin/notary signatures,
mixed invalid response filtering, no unrelated cache writes, rejection of legacy
unproven cache for HTTP and events, current-key HTTP success, retired-key HTTP
rejection, historical event success, and resistance to retirement rollback by
document replay. The existing key-expiry and programmable-notary regressions
remain enabled with genuinely signed fixtures.

The tests are not a production performance benchmark, external notary deployment
test, TLS/CA infrastructure audit, or proof of arbitrary database compromise
resistance. Test execution and measured results must be recorded from the actual
CI/test run rather than inferred from these assertions.

Protocol references: [Matrix key retrieval](https://spec.matrix.org/latest/server-server-api/#retrieving-server-keys)
and [JSON signature checking](https://spec.matrix.org/latest/appendices/#checking-for-a-signature).
