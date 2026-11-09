# Dependency and federation release gates

The dependency audit blocks RustSec vulnerabilities and unsoundness by default,
including transitive unsound dependencies. `notice` and `unmaintained` findings
remain visible warnings; they describe maintenance debt rather than establishing
an exploitable vulnerability. Bans, licenses and source restrictions remain active.

[PR #625](https://github.com/palpo-im/palpo/pull/625) lowered all advisory classes
to warnings after its lockfile's `pulldown-cmark 0.13.5` matched
[RUSTSEC-2026-0433](https://rustsec.org/advisories/RUSTSEC-2026-0433.html), a
quadratic parsing issue when `ENABLE_WIKILINKS` is enabled. The current lockfile
uses fixed `0.13.6`; it needs no exemption. The previous permanent `paste`
unmaintained exemption is also removed so its maintenance status is visible.

Enforcing unsoundness for transitive dependencies also exposed
[RUSTSEC-2026-0097](https://rustsec.org/advisories/RUSTSEC-2026-0097.html) in
`rand 0.7.3`, pulled in only by unused `textnonce` declarations. Its documented
logger re-entrancy condition requires `rand`'s optional `log` feature, absent
from this project's all-features dependency graph. Rather than grant an
exemption, this change removes the unused declarations and their obsolete
dependency chain. The ordinary `rand` dependencies already use patched releases.

Any future exemption must be an exact RustSec ID in `deny.toml`, with a matching
reason and an owner and `expires` date in
`tests/security/advisory-exceptions.json`. Expiry is exclusive, evaluated by UTC
calendar date. Missing metadata, broad crate exemptions, duplicate IDs, expired
records and metadata with no matching ignore fail before cargo-deny runs. The
policy validator also requires advisory schema version 2 and `unsound = "all"`;
switching to workspace-only or disabled unsoundness is rejected.
There are currently no exemptions. Add an exception only after assessing the
actual dependency path, exploit conditions and available upgrade; do not lower
the severity of an entire diagnostic class to make CI green.

```sh
python3 tests/dependency_policy.py
cargo deny check --warn notice --warn unmaintained
```

## Full Complement suite

The full suite no longer accepts a percentage of failures. Every failing test,
failing package and nonzero Go exit blocks CI. Failure exceptions are prohibited.
A skip must match an exact package, test path, terminal action and maximum
occurrence count in `tests/complement/suite-baseline.json`, with ownership, reason and expiry.
A passing case that was previously skipped is allowed. A new skip, skip becoming
a failure, an extra occurrence, or a failing descendant of an allowed parent is
rejected. Parent and child failures are both counted and neither can grant a waiver.

The initial baseline comes from the successful
[PR #637 run](https://github.com/palpo-im/palpo/actions/runs/42698383528): raw
**692 pass / 0 fail / 14 skip** across four package invocations. All fourteen
skips are individually documented, with no failing-test exceptions. These raw
counts retain parent events; they are not replaced by a leaf-only denominator.
The parser scopes duplicate detection to each package invocation and counts
every invocation, so identical names in distinct completed runs are not merged.
Missing test starts/terminals, duplicate or contradictory terminals, truncated
packages, missing packages, malformed records and unexplained process exit codes
fail closed. Timeout/cancellation is never an accepted known failure.

The independent `required_pass` gate remains stricter: every required case and
its descendants must explicitly pass, even if someone adds a suite exception.
`require_clean_suite` additionally rejects all failures and nonzero Go exits.

```sh
python3 tests/complement_baseline.py /tmp/complement/results.jsonl \
  tests/complement/suite-baseline.json \
  --revision 78516f9174a7c50c9a35a158fe88d16c5d9b88f5 \
  --exit-code-file /tmp/complement/exit-code
```

Update a baseline only from complete raw package/run/terminal records for the
same pinned Complement revision. Review each exception separately; expiration
requires a new assessment, not an automatic extension. Fixes should remove debt.

## Mixed peers and evidence

The existing required mixed lane now fetches Synapse by commit
`217e4c0f48e01db000127ed9bfb5251eefc51a2c`, alongside the pinned Complement
commit. It runs the same strict manifest in both directions; no additional
pipeline is introduced. The selected Synapse commit is inferred to be the
default-branch head fetched by PR #637 from that run's clone time and upstream
commit history; the old run did not directly record its Synapse SHA. This PR's
new run records the actual checked-out SHA before testing it.

All three federation jobs retain provenance with the checked-out Palpo and
Complement commits, applicable Synapse commit, Palpo image ID, executable SHA-256,
feature selection, fixture configuration hashes and Go version. Mixed artifacts
also retain direction-specific image IDs and run configuration. Cargo.lock-only
changes trigger the federation workflow. Pins improve source repeatability;
OS packages, base images and toolchain installation are not claimed to be fully
hermetic.

## Isolated Rust integration tests

The review cases run by exact registered name, in individual processes and fresh
databases under both default and all features. The ignored embedding startup
case also runs in both modes with its distinct
`PALPO_EMBEDDING_TEST_DATABASE_URL`; running an ordinary all-features suite does
not count an ignored integration test as executed.

Cargo's shared `database_` filter matches anywhere in a qualified test name.
CI checks the discovered selection for isolated `::review_` cases before running
that batch, and requires isolated case names to avoid `database_` altogether.
These cases own process-wide configuration and pools as well as their database.
