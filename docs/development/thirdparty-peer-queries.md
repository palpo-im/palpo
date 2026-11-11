# Third-party lookup peer boundaries

The six client third-party lookup routes use the configured application service's
`hs_token` in a fresh Bearer Authorization header. The legacy query token remains
for older AS deployments and has the same value. Client Authorization headers,
request bodies and all client `access_token` query parameters are excluded.
Protocol-specific custom fields are decoded and encoded as query values.

This follows the [Matrix v1.12 AS authorization contract](https://spec.matrix.org/v1.12/application-service-api/#authorization),
which requires the dedicated Bearer header and permits the legacy query form for
compatibility. It changes only requests made by these lookup handlers; it does
not establish that every other application-service operation uses the same policy.

A lookup response contributes results only after HTTP 200 and successful JSON
decoding. A 500 response with a valid success-shaped body is still a failure.
The protocol-specific metadata route continues to later registered peers after
status, transport or decoding failure. Mapping queries preserve healthy peer
results when another peer fails. No usable metadata/mappings returns the existing
`M_NOT_FOUND` response; an empty protocol inventory is `{}`. Unknown protocol
names do not cause an AS request.

Standard nonempty AS protocol instances contain `desc`, `fields` and `network_id`.
The optional `instance_id` extension is accepted and preserved when present,
without being required or invented for standard responses. The reference is the
[v1.12 protocol schema](https://github.com/matrix-org/matrix-spec/blob/v1.12/data/api/application-service/definitions/protocol.yaml).

## Regression evidence

`routing::thirdparty_peer_tests::review_thirdparty_queries_isolate_failed_peers_and_preserve_instances`
is an ignored test requiring a separate empty PostgreSQL database and its own
process with `PALPO_TEST_DATABASE_URL` set. It starts three real loopback AS HTTP
listeners, with unique configured credentials and deterministic registration IDs:

- The first returns HTTP 500 with otherwise valid metadata or mapping JSON.
- The second returns HTTP 200 with invalid JSON.
- The last returns healthy protocol metadata and location/user mappings.

The case exercises all six client routes through the real root router and checks
healthy results, dedicated credentials in both supported forms, discarded client
credentials/body, preserved custom fields, standard nonempty instances and the
optional instance extension. It also checks unknown protocols, dynamically
disabling/re-enabling the healthy bridge, and successful empty mapping responses.
The existing single-peer capability contract continues to cover normal AS
discovery and disabled registrations independently.

These are regression assertions, not claims of executed tests. Record the actual
test/CI result when reporting validation. The fixtures contain one usable peer;
they do not establish metadata conflict resolution, instance merging, duplicate
mapping removal, many-peer concurrency/admission limits, or progress while a peer
never responds. Calls remain sequential with the existing outbound timeout.
Those policies and load/fault scenarios remain separate follow-up work. Broad
MSC, build-feature and Spaces permission/topology coverage is also unchanged.
