# Outbound HAFleet transport v2

New registrations default to outbound transport. HAFleet initiates all transport
connections to Palpo over HTTPS. Palpo's homeserver sends Matrix App Service
transactions only to the colocated web service. The web service never calls an
outbound fleet's callback for capabilities, requests, probes or status.

See the [deployment guide](README.md) for the companion container and reverse
proxy configuration. Verify a real Matrix event round trip after deployment;
fixture test results do not establish a live installation's readiness.

## Configuration and deployment order

- `PALPO_TRANSPORT_ORIGIN`: a fixed public HTTPS origin. Route `/api/fleet/v2/*`
  from this origin to the web service, preserving `Host` including its port. It
  can share `PUBLIC_ORIGIN`, in which case proxy the whole web application.
- `PALPO_RELAY_ORIGIN`: a fixed origin reachable from the homeserver, such as
  `http://palpo-web-admin:8090` on the shared Docker network.
- `PUBLIC_ORIGIN`: the browser application's origin. Its existing Host,
  same-origin and CSRF checks still apply.
- `PALPO_CALLBACK_ORIGINS`: the explicit legacy callback allowlist. It is not
  consulted for outbound destinations, which only the operator configures.

The Compose example requires both public origins explicitly. Configure the
public reverse proxy and verify the homeserver can reach the relay on the shared
Docker network. Build the image with `docker compose -f deploy/compose.yaml build`
to stage it without replacing a running container. Activate the same source
snapshot with `docker compose -f deploy/compose.yaml up -d --wait` after the
matching HAFleet version and proxy routes are ready.

Install a Palpo server version supporting atomic
`PUT /_palpo/admin/v1/appservices/{id}/url` before migrating an existing fleet.
The JSON body is `{ "url": "...", "expected_url": "..." }`; only the URL
changes. The web service reads back and verifies the complete registration.
It never uses DELETE/re-registration or treats a conflict as success. If a
previous attempt already changed the URL, a full matching read-back recovers it.

Deploy the web service, then explicitly migrate an existing fleet using the
administrator UI or `POST /api/fleets/{id}/outbound` with `{ "requestId": "..." }`.
The operation is persisted and retryable. Fleet, registration, representative,
Agent, project, request and Matrix event identities remain unchanged. Every
existing request with a saved source event is replayed into the new generation
so HAFleet can resume status publication using its canonical idempotency checks.

The owner downloads the new configuration, imports it into HAFleet, and verifies
the initial connection once. Only after outbound acceptance with all reverse
forwards disabled should the operator remove the old reverse route. Existing
legacy fleets continue using their explicit callback transport until migration.

## Owner configuration

The existing owner-bound pairing/download response adds:

```json
{
  "transport": {
    "mode": "outbound",
    "url": "https://palpo.example/api/fleet/v2/hf_0123456789abcdef0123456789abcdef",
    "token": "<independent fleet machine secret>",
    "generation": 1
  }
}
```

The registered App Service URL is the fixed relay origin plus
`/api/relay/v2/{fleetId}`. Existing `as_token` and `hs_token` retain their Matrix
roles. The machine token is separate and absent from catalog, list, status,
audit and migration projections. Machine requests require `Authorization:
Bearer <token>` and `X-HAFleet-Generation: <generation>`. Browser Origin headers
are refused on machine routes; browser cookies do not authorize them.

## Delivery and receipt protocol

`GET {transport.url}/poll?lane=matrix|work&consumer=<stable-UUID>&wait=25000`
returns:

```json
{
  "v": 2,
  "generation": 1,
  "delivery": {
    "id": "stable-delivery-id",
    "lane": "matrix",
    "token": "lease-receipt-token",
    "expiresAt": "2026-09-09T00:00:30.000Z",
    "kind": "transaction",
    "payload": { "transactionId": "original-Matrix-transaction-id", "body": { "events": [] } }
  }
}
```

An empty poll has `delivery: null`. Wait is between 0 and 25000 ms. Each lane has
one active 30-second lease, in FIFO order. Another poll cannot replace an active
lease; expiry makes the same delivery eligible with a new receipt token.
`POST /ack {id,lane,token}` succeeds only for the current generation and token.
Repeating the same completed ACK succeeds; expired/replaced leases return
`409 stale_lease`. A delivery ACK means HAFleet has persisted receipt, not that
it approved, allocated or completed business work.

Work deliveries use `kind: "probe"` with the existing v1 probe body, or
`kind: "request"` with the original v1 request plus its `sourceEventId`.
Private owner-room bindings travel only on this authenticated work lane.
Full Matrix transactions, including device/ephemeral fields when present, are
stored before the relay returns HTTP 200. Their original ID and content digest
make retries idempotent across machine generations of the same AS registration;
changed content returns 409. An old ACKed transaction is not queued again after
rotation, so HAFleet must retain its durable AS inbox independently of its machine
credential generation. User queries acknowledge
only the exact representative or registered managed identities within this
fleet's namespace. Unknown users and aliases return 404.

SQLite persists queue records, receipt leases and completed tombstones. Defaults
are 1000 pending records, 10000 retained records, and 16 MiB of pending payload
per fleet. Bodies are limited to 1 MiB and 1000 Matrix events. Capacity exhaustion
returns `503 queue_full`; nothing is silently dropped or automatically evicted.
Completed payloads are removed while digest/receipt tombstones remain. Old
generation records remain isolated. Retained-history capacity currently requires
explicit operator maintenance or a configured capacity increase; automated
tombstone compaction is not implemented.
The operator can configure positive integer limits with
`PALPO_FLEET_QUEUE_MAX_PENDING`, `PALPO_FLEET_QUEUE_MAX_RECORDS`, and
`PALPO_FLEET_QUEUE_MAX_BYTES` before starting the service. The deployment
Compose configuration passes these settings into the container.

Administrators can inspect **Inspect delivery capacity** or authenticated
`GET /api/fleets/{fleetId}/outbound`, returning only
`{queue:{records,pending,bytes,limits:{records,pending,bytes}}}`. Monitor retained
records as well as pending work; ACKs free payload capacity but retain deduplication
records. Before reaching the limit, take a protected backup of the stopped
service's existing SQLite volume and redeploy that same volume with a larger
`PALPO_FLEET_QUEUE_MAX_RECORDS` (for example `100000`). Keep the same public and
relay origins and verify the queue counters/limits after restart. A full queue's
original deliveries remain recoverable and retry after capacity increases. Do not
SQL-delete receipts or replace the database to clear capacity: arbitrary Matrix
transaction IDs need their tombstones to reject changed or repeated old content.
This release uses explicit storage expansion rather than a retention policy that
could silently re-execute old Matrix transactions.

## Monotonic publication and readiness

`POST /updates` accepts:

```json
{ "v": 2, "generation": 1, "sequence": 1, "heartbeat": true,
  "capabilities": "<optional v1 capabilities object>",
  "statuses": ["<up to 200 optional v1 request status objects>"],
  "probeReceipts": ["<up to 10 optional v1 exact probe receipts>"] }
```

The sequence is a positive safe integer and must increase. Replaying the last
sequence with canonically identical content succeeds without refreshing liveness.
Older sequences return `stale_sequence`; changed same-sequence content returns
`sequence_conflict`. Validation finishes before committing any part of an update.
Only pre-existing requests in this fleet with matching original role, quota,
source, target and Agent definition can be updated. Namespace checks precede
storage. Usability and managed identity linkage additionally require actual
Matrix membership, a current-generation connection proof and a recent heartbeat.
Each status carries `observedAt`, the ISO time when HAFleet actually checked the
local request. A usable request needs a current-generation status both observed
and received within the last 90 seconds. Up to five seconds of clock skew is
allowed, capped at server receipt time. Missing/invalid timestamps and clocks
further in the future are stored as unverified observations; they do not reject
the update or block heartbeats. A frozen update delivered late cannot revive its
old ready state. Heartbeats and browser reads never extend either lifetime;
clients must publish active observations within that window even when batching.
Admission failures can remain `submission_pending`, never synthetic fulfillment.

A heartbeat alone cannot establish the initial connection. Owner verification
creates the real reception/probe event and queues its exact challenge; HTTP 202
means it is awaiting receipt. HAFleet first persists and ACKs the original Matrix
transaction, then processes it and publishes the probe receipt. Palpo matches the
exact event, room, challenge, generation and ACKed transaction and rechecks that
the owner and representative are still joined. This establishes the generation's
connection proof. Repeated Verify actions reuse the same generation's probe so
an uncertain durable publication remains retryable; rotation alone resets it.
Heartbeats then maintain online status for 90 seconds without
a browser or periodic owner renewal. Offline established fleets remain eligible
for durable request queuing; the UI identifies their last published resources
and shows queued delivery separately from allocation.

`POST /api/fleets/{id}/outbound {requestId,rotate:true}` explicitly rotates the
machine secret and increments the generation. Outstanding Matrix deliveries and
all existing requests are retained/replayed; old leases, updates and credentials
cannot act in the new generation. A new exact Matrix proof is required. Paused
and revoked services reject both machine and relay credentials.

## Validation limits

Validation on 2026-09-08: 57 Node tests and all three Chromium suites pass.
Node and Playwright fixtures exercise the actual HTTP routes and a file-backed
SQLite restart, without contacting a live deployment or a model. They cover delayed/expired
leases, duplicate transactions and ACKs, full queues, state/sequence recovery,
token isolation, generation rotation, migration CAS recovery, SQLite failure
rollback, stale status expiry, repeated owner verification, offline request
retry, immutable status bindings and actual room membership. The browser check
exercises default outbound authorization, owner download, exact relay proof,
offline resource selection and queued request delivery while failing on any
reverse HAFleet call. Encrypted transaction payloads are retained unchanged;
full native encrypted chat/file acceptance requires the coordinated HAFleet and
Robrix deployment and is not established by the web fixtures alone.
