#!/bin/bash
#
# Runs Complement tests against Palpo.
#
# Usage:
#   ./tests/complement/complement.sh [test_filter]
#
# Arguments:
#   test_filter  Optional Go test filter pattern (e.g. "TestLogin")
#
# Environment variables:
#   COMPLEMENT_SRC   Path to complement source (default: clones to /tmp/complement)
#   RESULTS_FILE     Path to write test results
#   COMPLEMENT_ARGS  Additional args to pass to complement
#   CI               Set to "true" to use Dockerfile.github instead of local Dockerfiles

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TEST_FILTER="${1:-}"
COMPLEMENT_SRC="${COMPLEMENT_SRC:-/tmp/complement}"

# Clone complement if not present
if [ ! -d "$COMPLEMENT_SRC" ]; then
    echo "--- Cloning Complement ---"
    git clone --depth 1 https://github.com/matrix-org/complement.git "$COMPLEMENT_SRC"
fi

cd "$REPO_ROOT"

if [ "${CI:-}" = "true" ]; then
    # CI mode: single Dockerfile, no base image dependency
    echo "--- Building complement-palpo image (CI mode) ---"
    docker build \
        -t complement-palpo \
        -f tests/complement/Dockerfile.github \
        .
else
    # Local mode: uses base + test Dockerfiles (with Chinese mirrors)
    echo "--- Building complement-palpo-base image ---"
    docker build \
        -t complement-palpo-base \
        -f tests/complement/Dockerfile.base \
        .

    echo "--- Building complement-palpo image ---"
    docker build \
        -t complement-palpo \
        -f tests/complement/Dockerfile.test \
        .
fi

cd "$COMPLEMENT_SRC"

RUN_ARG=""
if [ -n "$TEST_FILTER" ]; then
    RUN_ARG="-run ${TEST_FILTER}"
fi

RESULTS_DIR="${RESULTS_DIR:-$COMPLEMENT_SRC}"

echo "--- Running Complement tests ---"
set +e
COMPLEMENT_BASE_IMAGE=complement-palpo \
    COMPLEMENT_ENABLE_DIRTY_RUNS=1 \
    go test -json -count=1 -timeout 60m \
    "${RUN_ARG}" \
    ${COMPLEMENT_ARGS:+"${COMPLEMENT_ARGS}"} \
    ./tests/csapi \
    ./tests \
    | tee "${RESULTS_DIR}/results.jsonl"
TEST_EXIT=$?
set -e

# Extract final test results (pass/fail/skip), sort by test name
echo "--- Generating sorted results ---"
grep '^{' "${RESULTS_DIR}/results.jsonl" \
    | jq -c 'select(.Test != null and (.Action == "pass" or .Action == "fail" or .Action == "skip"))' \
    > "${RESULTS_DIR}/_raw_results.jsonl"
# Sort: fail first, then skip, then pass; within each group sort by test name
jq -sc 'sort_by(
    (if .Action == "fail" then 0 elif .Action == "skip" then 1 else 2 end),
    .Test
  )[]' "${RESULTS_DIR}/_raw_results.jsonl" > "${RESULTS_DIR}/__test_all.result.jsonl"
rm -f "${RESULTS_DIR}/_raw_results.jsonl"

echo "=== Test Summary ==="
echo "Total: $(wc -l < "${RESULTS_DIR}/__test_all.result.jsonl")"
echo "Pass:  $(grep -c '"pass"' "${RESULTS_DIR}/__test_all.result.jsonl" || true)"
echo "Fail:  $(grep -c '"fail"' "${RESULTS_DIR}/__test_all.result.jsonl" || true)"
echo "Skip:  $(grep -c '"skip"' "${RESULTS_DIR}/__test_all.result.jsonl" || true)"
echo "Results: ${RESULTS_DIR}/__test_all.result.jsonl"

exit $TEST_EXIT
