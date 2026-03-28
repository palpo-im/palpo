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

echo "--- Running Complement tests ---"
set +e
COMPLEMENT_BASE_IMAGE=complement-palpo \
    COMPLEMENT_ENABLE_DIRTY_RUNS=1 \
    go test -v -count=1 -timeout 60m \
    ${RUN_ARG} \
    ${COMPLEMENT_ARGS:-} \
    ./tests/csapi \
    ./tests \
    2>&1 | tee "${RESULTS_FILE:-/dev/stdout}"
TEST_EXIT=$?
set -e

if [ $TEST_EXIT -ne 0 ]; then
    echo "--- Complement tests finished with failures (exit code: $TEST_EXIT) ---"
else
    echo "--- Complement tests passed ---"
fi

exit $TEST_EXIT
