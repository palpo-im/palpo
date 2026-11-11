# Verified key history: measurements and recovery invariants

Verified key history retains signed documents because a document that is no
longer current can still be needed to verify an older event. A retirement record
also constrains older copies that called the same key current. Deleting entries
solely because `valid_until_ts` is in the past can discard either property.
This change does not install an automatic archive or pruning policy.

## Reproducible fixture

Run this exact ignored case alone, against a separate **empty, dedicated**
PostgreSQL database. The fixture owns process-global configuration and modifies
its own key tables. CI runs a fresh process/database with default and all features.

```text
cargo test --locked -p palpo --lib server_key::history_capacity_tests::review_verified_history_capacity_and_snapshot_restore -- --ignored --exact --test-threads=1 --nocapture
```

Use the same command with `--all-features` before the test name for the other
feature mode. Record the source commit, binary hash, Rust version, PostgreSQL
version, CPU/memory, build profile and host load alongside the emitted evidence.
Execution results must come from a completed test run; the assertions below are
not measured performance claims.

The workload grows one server's history from 500 to 2,000 genuinely signed raw
documents. Each document contains an active key, a distinct indexed key ID, and a
128-byte signed extension. The first document additionally contains an ancient
event key and a key subsequently retired by two explicit signed declarations.
The synthetic keys are trusted fixture input, with no TLS or notary bootstrap
simulation. Network trust is covered by the separate dual-TLS-peer notary test.

At each size the case performs 32 samples per lookup, alternating case order:

| Lookup | Evidence asserted |
| --- | --- |
| Current HTTP key | Valid latest key; exactly one raw document loaded and verified. |
| Ancient event key | A valid V5 event signed using the first document's key; one matching raw document loaded. |
| Common event key | A valid V5 event whose key occurs throughout history; no more than the online budget of eight raw documents loaded. |
| Retired HTTP key | Refusal without loading historical bodies. |
| Event before retirement | Successful V5 signature verification, including authenticated retirement bounds. |

`T06_HISTORY_MEASUREMENT` prints p50/p95/p99, all observed selector query counts,
fixture timestamp and pool size. Quadrupling stored history must not add database
round trips. This is a round-trip and deserialization/verification bound, **not**
a constant database-work claim. For example, a common-key lookup may scan and
sort many matching rows before its limit, while the retirement aggregate must
consider matching declarations. Query plans expose that distinction.

`T06_HISTORY_PLAN` prints full PostgreSQL `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`
for latest-document, ancient/common event-key and retirement queries. The explain
fixtures are checked against SQL observed from the actual selectors and use the
same typed bind values. No planner switch forces an index, and no specific plan
or timing is a test pass condition. Read each node's `Actual Rows` together with
`Actual Loops`; parent and child rows describe overlapping work and must not be
summed as unique physical rows. Buffer counters on a parent also include its
children. See [PostgreSQL EXPLAIN](https://www.postgresql.org/docs/18/sql-explain.html).

`T06_HISTORY_STORAGE` reports document/version relation and index bytes from the
dedicated database, including PostgreSQL page overhead. It is not a per-document
storage promise. The fixture is small, warmed, local and sequential; it does not
establish production concurrency, CPU/RSS bounds, a throughput target, cold-cache
behavior or an SLO. Latency has no pass/fail threshold.

## Logical recovery round trip

The test captures both key tables in **one SQL statement snapshot**, serializes
the complete rows to JSON, and records the byte count and SHA-256. A transaction
then deletes the fixture server's rows and restores the raw-document rows before
the version-index rows. The existing constraints and indexes remain in place.
It does not call the normal cache `persist` path during restoration: doing so
would refresh `validated_at` and could extend a cached key's seven-day bound.

The recovered rows must exactly match the snapshot, including IDs, raw signed
JSON, canonical document digest, source/source server, validation policy version,
observation time, key selectors, expiry and retirement flags. Actual current HTTP
key lookup and historical event signature verification run again; retired HTTP
keys and events at/after retirement remain rejected. A deliberately broken index
reference must fail its foreign-key check and roll back without deleting the
previous valid history. `T06_HISTORY_RESTORE` records the verified round trip.

This is an in-process **logical data round trip in an existing dedicated schema**.
It does not rebuild schema/index definitions, reset sequences, restart PostgreSQL,
kill a restore process, restore a full Palpo database, recover from storage loss,
or exercise object storage. These operations remain separate recovery drills.

## Archive and production restore requirements

An archive design must preserve the entire signed raw JSON and its validated
provenance/observation metadata, plus key-index consistency and explicit
retirements. Store an independently protected manifest containing schema and
validation versions, source identity, row counts, snapshot boundary and integrity
digests. A checksum detects accidental changes but does not authenticate an
untrusted archive. Do not silently promote legacy `server_signing_keys` rows or
re-import old documents through a path that marks them freshly verified.

Retention needs a defined historical-event horizon and an authenticated online
record of retired keys. Measure distributions by server and key, document sizes,
indexed query plans, backup size/duration and concurrent request load before
choosing a capacity or pruning limit. The eight-document online load bound alone
does not justify deleting older signed documents. Keep a recoverable original
until a proposed policy has demonstrated historical verification and retirement
semantics after restore.

For a real PostgreSQL backup, use a reviewed full-database procedure that includes
schema, indexes, sequences, migration ledger, ownership and the rest of Palpo's
data. Restore into a separate compatible environment, validate provenance and
constraints, run `ANALYZE`, and exercise current and historical key use before
cutover. PostgreSQL documents consistent logical dumps, error handling and
single-transaction restores in its [SQL dump guide](https://www.postgresql.org/docs/18/backup-dump.html).
The test fixture's JSON format is not a supported production import interface.
