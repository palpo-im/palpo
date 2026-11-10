# Outbound diagnostics and reproducible recovery drills

`GET /diagnostics` and the embedded `MatrixServer::diagnostics()` API expose
bounded, process-local operational counters. HTTP responses use `Cache-Control:
no-store` and return 200 with component status in the body. Use `/ready` for the
readiness HTTP status; `/health` remains liveness. Protect diagnostics at the
deployment ingress if queue volumes and capacity are private operational data.
No destination, user identifier, token, database URL, or raw error text is a
metric label. These are structured JSON diagnostics, not a Prometheus endpoint.

## Reading the measurements

| Field | Meaning |
| --- | --- |
| `queue.snapshot.total_rows` | Durable requests present at one PostgreSQL statement snapshot. |
| `queued_rows` | Rows in `created` state, waiting to be claimed. |
| `claimed_rows` | Rows in `pending` state, including durable requests awaiting a retry. This is not the number of live network calls. |
| `retry_marked_rows` | Subset of durable rows whose persisted retry count is positive, overlapping the state counts. |
| `unknown_age_rows` | Legacy rows with no trustworthy enqueue timestamp. |
| `future_timestamp_rows` | Rows whose enqueue time is after the sampler's database clock; investigate clock corrections or imported data. |
| `oldest_known_age_ms` | Database time minus the earliest known enqueue timestamp, clamped to zero on future dates and saturated at signed 64-bit maximum. Null when no known timestamp exists. |
| `worker.heartbeat_age_ms` | Time since the worker's actual scheduler loop progressed; an idle worker beats every five seconds. Null before the first worker start. |
| `worker.recovering_for_ms` | Time since startup or the first observed worker/sweep failure, until successful durable recovery; null after recovery. |
| `completed_recoveries`, `last_recovery_ms`, `longest_recovery_ms` | Startup/recovery completions and elapsed durations within this process. Startup counts once. |
| `query_pool_wait`, `coordination_pool_wait` | Global data-layer pool checkout waits, counting completed, failed, and dropped/cancelled waits. Prepared startup pools and direct pool consumers are outside this measurement. |
| `outbound_permit_wait` | Shared outbound semaphore wait time, including cancelled waits and closed-semaphore failures. |
| `outbound` | Three fixed kinds: federation, appservice, push; active `send_events` calls, elapsed time, and fixed outcome families. Preparation before `send_events` and database ACK errors after it are outside this call boundary. |
| `federation_deadline_exceeded` | Outer federation request deadlines after a permit has been acquired; permit wait is measured separately and is outside that deadline. |

Latency bucket boundaries are supplied as `latency_bucket_upper_us`; the seven
`bucket_counts` are **disjoint**, with an overflow bucket after the six inclusive
upper bounds. Counters saturate on overflow. Relaxed atomic snapshots are
approximate under concurrent updates; sums may briefly differ while a sample is
being recorded. They are unsuitable for exactly-once accounting. All counters
and heartbeat/recovery times reset on process restart, whereas queue rows and
enqueue timestamps are durable. Aggregate each process's rates independently.
Queue counts, worker state, pool snapshots, and counters are sampled at different
instants and are not one transactionally consistent global snapshot.

Outcome names are supplied once in `outcome_names` and index each
`outcome_counts` array: success, storage, transport timeout, transport, decode,
Matrix, application, cancelled. Remote non-200 appservice responses currently
fall under application. Classification follows Rust error families, not exact
remote HTTP statuses; no arbitrary remote strings become labels. A successful
send call can still be followed by an ACK failure and a later duplicate delivery.
The delivery contract remains at least once.

## Sampling cost and failure semantics

Queue aggregation runs at most once per **30 seconds per process**, with one
refresh admitted at a time and no refresh wait queue. It has a **one second**
budget covering pool checkout and query, and a **500 ms** PostgreSQL transaction
statement timeout. `/ready` never runs this aggregate. The full diagnostics call
also performs the existing one-second readiness probe, so allow up to two seconds
for an admitted call. Most cached calls need only the readiness query.

Exact counts can scan a large backlog. The combined count/age aggregation has no
extra enqueue index or per-enqueue index maintenance. Do not
increase scrape frequency expecting fresher counts. If statement or checkout
budgets are exceeded, the previous successful snapshot and its increasing
`snapshot_age_ms` remain visible. State reports `timed_out` or `failed` for the
attempt, then `stale` during its TTL. No previous sample means `snapshot: null`;
it never means an empty queue. `busy` means a concurrent refresh already owns
the single sampling slot. `not_initialized` means no installed query pool.
A cancelled refresh also marks cached data stale until a successful refresh.

Suggested alert inputs: missing/stale samples; sustained oldest known age or
queued/retry growth across multiple fresh samples; heartbeat age materially
above the five-second idle interval; increasing recovery duration, permit wait,
checkout cancellations, transport failures or deadline rate. Set thresholds from
the deployment's normal traffic and recovery objectives. Unknown legacy ages
must be shown alongside known oldest age; an unknown backlog cannot establish an
age SLO. These inputs do not install or validate an external alerting system.

## Timestamp migration and rollback

Migration `2026-11-10-010000_outgoing_enqueue_time` adds nullable `enqueued_at`
milliseconds, then separately installs a PostgreSQL `clock_timestamp()` default.
Existing rows stay null rather than receiving a fabricated migration-time age.
New inserts from both updated and older binaries receive a real timestamp when
they omit the column; the model deliberately omits it from its insert shape.
Claiming, failure and retry updates preserve it. Explicit null imports remain
unknown. The migration uses ordinary table DDL; schedule the schema change
for an appropriate deployment window on very large queues. Roll back application
binaries first if dropping the new column: the new reader requires that column.
The down migration discards recorded ages but preserves request rows.

## Regression and evidence

Ordinary tests cover fixed histogram buckets, cancellation accounting, bounded
outcome labels and uninitialized diagnostics. The following two **ignored** tests
must each run alone, in a fresh test process against a separate **empty,
dedicated** database with `PALPO_TEST_DATABASE_URL` set. They change schema and
terminate their own PostgreSQL session. Never point them at live data.

```text
cargo test -p palpo --all-features observability::tests::review_outbound_diagnostics_tracks_queue_pool_failures_and_restored_rows -- --exact --ignored --nocapture --test-threads=1
cargo test -p palpo --all-features observability::tests::review_outbound_process_restart_preserves_requests_and_reports_recovery -- --exact --ignored --nocapture --test-threads=1
```

The first prints comparable before/fault/restored queue counts: empty 0; backlog
4 (queued 3, claimed 1, retry marked 1, unknown 1); schema failure and exhausted
pool retain the previous 4; restored sampling returns 4. It checks a genuinely
terminated PostgreSQL backend reconnects, plus checkout and permit cancellation,
the legacy/new-row migration boundary and HTTP diagnostics/readiness contracts.
Schema unavailability and exhausted capacity are distinct from network failure.

The second launches the current test executable twice. Child one commits three
pending PDU request rows and starts the real supervisor. A loopback appservice
refuses its first transaction with HTTP 503, then holds the retried transaction
open after the real 30-second backoff. Child one records three retry-marked rows,
an active outbound attempt, an application error and a fresh scheduler heartbeat;
the parent forcibly kills the child while that retry is in flight.
Child two initializes fresh globals, verifies the same durable row IDs,
starts the supervisor, resends the same payload, receives HTTP 200 and ACKs all
three rows. The parent gateway survives both children. Output includes original
row IDs, fault and final queue counts, three observed transactions and recovery/heartbeat
measurements. A task abort is not used as a substitute for process termination.

Test output is the source of measured evidence; these expected values are
assertions, not benchmark results. Record the CI run and printed values when
reporting a successful drill. These tests do **not** measure PostgreSQL service
shutdown/restart, a real network partition, disk loss, an external alert firing,
long-duration load, production throughput, percentile latency or an improvement
percentage. Those require a separately controlled environment and workload.
