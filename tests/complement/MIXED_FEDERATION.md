# Mixed federation coverage

The mixed job runs the same manifest twice: hs1=Synapse/hs2=Palpo/hs3=Synapse and
hs1=Palpo/hs2=Synapse/hs3=Palpo. Most cases deploy two peers; ACL and restricted
failover cases deploy three. A successful local request is insufficient: tests must
observe the response from the other implementation or the event in its sync.
The manifest is `mixed-cases.txt`; it drives both selection and the results gate.

## Audit

This audit compares the previous 15 selected top-level tests with Palpo's
outbound calls in `directory`, `routing/client/profile`, `room/alias`,
`room/space`, `membership`, `user/key`, `media/remote`, and event fetching.
The Complement API and upstream cases were reviewed at
`78516f9174a7c50c9a35a158fe88d16c5d9b88f5`, pinned in the workflow. Synapse is
still built from its current default branch so compatibility changes are visible.

| Area | Previous mixed coverage | Coverage in the manifest |
| --- | --- | --- |
| Federation authentication | Indirectly exercised by joins/messages | Explicit rejection of unsigned POST publicRooms; authenticated client queries must succeed against that same peer |
| Public room discovery | None; upstream TestPublicRooms deploys one server | Remote GET/POST, metadata, private-room exclusion, limit, complete pagination without duplicates, name/topic/no-match filters, unpublishing, invalid remote pagination errors |
| Remote profile lookup | None; upstream profile federation tests use a simulated peer | Full profile, displayname and avatar lookup before sharing a room, Unicode values |
| Alias resolution | Unicode lookup | Unicode resolution and actual join by alias, missing/deleted alias returning M_NOT_FOUND |
| Membership | Invite, join by ID, candidate failover, invite rejection, ban/unban | Also remote leave/reinvite/rejoin, kick, knock and knock-restricted transitions; restricted joins, local-user remote authorization and three-peer restricted/knock-restricted candidate failover |
| Server ACLs | None | Three-peer PDU and typing/receipt EDU deny rules, with a separate allowed room as a positive delivery control |
| State and profile changes | Incidental state in message/join tests | Incremental remote topic and membership displayname/avatar updates, checked again via state retrieval |
| PDUs | Messages and backfill | Also direct event retrieval, redaction observed on the peer, opaque encrypted-event payload integrity |
| Receipt/typing/to-device EDUs | Typing and to-device | Also m.read receipt observed on the peer's incremental sync |
| Keys/devices | Device list changes and key upload/query | Retained; upstream key case already checks OTK claim and exhaustion, signatures and device displayname |
| Media | Authenticated federation content | Retained TestContentMediaV1; this is not a claim of thumbnail/legacy-route coverage |
| Spaces | Restricted-room hierarchy | Also public remote child traversal, suggested_only and private-child exclusion |
| History | Federated message history/backfill | Retained TestMessagesOverFederation, including its visibility cases |
| Presence | Remote presence | Retained TestRemotePresence |

The six Palpo-owned top-level tests live in `mixed/` and are copied into
Complement's `tests/palpo_mixed` package. Ten additional upstream membership/ACL
tests are selected. Every default run must report all 31 manifest entries as
passed in both directions. Subtest counts are not used as a coverage percentage.

The Synapse fixture explicitly enables `allow_public_rooms_over_federation` in
`Dockerfile.synapse`. Its default is false: publishing a room alone is not enough
to permit remote discovery. This setting does not disable federation signatures;
the unsigned-request test verifies that the configured peer still rejects them.
Malformed directory tokens currently produce 400, 500 or 502 depending on the
serving/querying peer; the error test requires a Matrix error and rejects
success/schema decoding. Positive directory assertions verify authentication.

POST requests use the standard `server` query parameter. The audit found that
Palpo previously read it only from the JSON body; the handler now extracts the
query parameter and gives it precedence, retaining the legacy body fallback.

## Remaining gaps

These areas need additional fixtures or known implementation work; they are not
silently classified as covered by an unrelated join/message test:

- Room-type-only directory filtering (`filter.room_types`), room network
  selection and backward pagination. The current local directory implementation
  does not apply `room_types`, and forwarding always selects the Matrix network.
  Add the matching regression cases alongside those fixes.
- Discovery through well-known delegation, SRV records, explicit ports, and
  DNS/TLS failures. Complement's fixed hs1/hs2 Docker routing does not model them.
- ACL wildcard/IP-literal edge cases and larger topology permutations. The
  selected ACL cases deny a named server and use a positive sentinel room;
  they do not verify every ACL matching rule.
- Malformed/mismatched signatures, expired signing keys, key rotation and
  notary failure. A programmable federation peer is needed; a successful join
  between two real servers cannot verify each rejection rule.
- State resolution across divergent graphs, rejected auth chains, missing-event
  recovery, and network partitions/restart durability. Upstream has simulated
  peer tests for several of these, but the selected real two-server cases do not
  cover the whole set.
- Legacy media routes, thumbnails, streaming failures and size limits.
- Client-side E2EE key verification, decryption, cross-signing and recovery.
  The encrypted event test checks only opaque server-to-server relay.
- Device deletion, cross-signing and key-query timeout/partial-failure handling.
- Remote timestamp-to-event lookup; existing history tests primarily exercise
  `/messages` and backfill, not that separate outgoing request path.
- OpenID token validation/expiry and third-party identity invitations, which
  require identity-service fixtures rather than another ordinary homeserver.
- Peeking and policy-signing extensions: Palpo exposes these request paths,
  but a stock Synapse peer does not establish support for their optional
  protocols. They need capability-specific peers and explicit contract tests.

Two existing unstable to-device subtests remain excluded: `stopped_server` in
both directions and `interrupted_connectivity` for Synapse -> Palpo. Every other
skip fails the gate. Their exact names are preserved with the results; a passing
parent is not presented as coverage of those excluded scenarios.

## Run and inspect

Build `complement-palpo` and `complement-synapse` with the workflow Dockerfiles,
check out Complement at the pinned revision, then run:

```sh
bash tests/complement-mixed-federation.sh /path/to/complement /tmp/mixed-results
python3 tests/mixed_federation_results.py /tmp/mixed-results
```

For a focused diagnostic run:

```sh
TEST_FILTER='^TestMixedPublicRooms$' DIRECTION=palpo-synapse \
  bash tests/complement-mixed-federation.sh /path/to/complement /tmp/directory-results
```

A custom TEST_FILTER disables the full-manifest requirement and is explicitly
logged as diagnostic. Test failures, unexpected skips, missing package completion
and nonzero process exits still fail. Extra skips require an explicit
newline-separated `ALLOWED_SKIPS` list, rather than being accepted automatically.

Artifacts include raw Go events, readable output, sorted results, process exit
status, required cases, allowed skips, Complement revision, image IDs and the
effective run configuration. Both directions must exist and complete successfully;
a compile failure or timeout cannot turn green merely because no failed test
event was produced. The results gate itself has Docker-free regression tests:

```sh
python3 -m unittest discover -s tests -p 'mixed_federation*_test.py'
```

The directory regression intentionally fails on code predating #521. Merge that
authentication fix before expecting a green default mixed run.
