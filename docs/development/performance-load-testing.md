# Timeline HTTP load probe

`tests/performance_load.py` exercises a real Palpo process and PostgreSQL over
loopback HTTP, using persistent connections and a closed-loop thread pool. It
covers `/versions`, history pages of 20 and 100 events, initial sync and idle
incremental sync at configurable concurrency. Setup sends 120 messages through
the real client API into a private room. Setup and warmup are outside timings.
`--include-writes` additionally sends unique transactions into a separate room,
so it leaves the read fixture's history unchanged. Keep the test server's
`rc_message` rate and burst high enough for the selected load and fixture setup;
rate-limit errors fail the probe rather than being retried or hidden.

Use an isolated database and a disposable `.test` server, with a local `owner`
account. Bind the server to `127.0.0.1`; disable federation and update checks.
The probe refuses remote HTTP origins, and prepare refuses a non-`.test` account.
The fixture contains an access token and must stay private and untracked.
Reports contain binary/probe SHA-256 hashes, successful request throughput,
nearest-rank p50/p95/p99, individual successful latency samples, errors, pool
capacities and query-pool telemetry deltas. Pool counters include background
activity and occasional diagnostics queue refreshes; they are not SQL counts.

Build using the workspace's ordinary Cargo configuration. Discover the artifact
directory with `cargo metadata --no-deps --format-version 1`; do not redirect
Cargo home, target or temporary directories. Keep frozen copies of each tested
binary and use exactly the same profile, configuration and fixture for both.
Do not build or run other benchmarks during timing intervals.

```sh
python tests/performance_load.py --prepare --password DISPOSABLE_OWNER_PASSWORD \
  --fixture target/performance/private-fixture.json \
  --binary target/performance/before --output target/performance/before-1.json \
  --requests 96 --concurrency 1 8 32

# Stop the first process and start the fixed binary against the same database.
python tests/performance_load.py \
  --fixture target/performance/private-fixture.json \
  --binary target/performance/after --output target/performance/after-1.json \
  --requests 96 --concurrency 1 8 32
```

Repeat in alternating binary order to expose machine and cache variation. Verify
that the supplied binary is the actual server executable. The probe records its
file hash but does not attest a remote process. The history-response hashes
normalize only `unsigned.age`; equal hashes compare event order, contents,
pagination and recipient metadata across runs. Inspect errors separately from
successful latency statistics; any measured error exits nonzero. Warmup failures
also fail the run.

This is a diagnostic read workload, not a production capacity estimate. It does
not measure distributed federation, large-room state, sustained writes,
multi-process operation or long-poll fanout. Debug builds help locate excess
database work, but publish capacity numbers only after optimized builds and
representative deployment workloads. More concurrent clients than query-pool
connections deliberately tests queueing pressure.

## Storage regression

`room::timeline::batch_tests::review_timeline_batch_preserves_events_and_bounds_storage_queries`
runs alone in an empty dedicated `PALPO_TEST_DATABASE_URL`. CI runs it with both
default and all features. It checks that a 20-event history page uses one
event storage query and that 260 event IDs use two batches. It compares batch
and single-event results and covers duplicate input, caller
ordering, outlier/soft-failure/backfill/rejection flags, trusted device provenance,
empty input, missing event bodies and lazily decoded malformed JSON.

Single-event reads join event metadata and JSON in one query. Timeline readers
fetch up to 256 event bodies at a time using the same projection. They apply
retention, client metadata hydration and visibility checks in timeline order.
