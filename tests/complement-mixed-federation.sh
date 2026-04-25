#!/usr/bin/env bash

# Run Complement federation tests with mixed homeserver implementations:
# one Palpo homeserver and one Synapse homeserver.
#
# Usage:
#   bash tests/complement-mixed-federation.sh <complement-src> <results-dir>
#
# Environment:
#   DIRECTION      synapse-palpo | palpo-synapse | both (default: both)
#   PALPO_IMAGE    Docker image for Palpo (default: complement-palpo)
#   SYNAPSE_IMAGE  Docker image for Synapse (default: complement-synapse)
#   TEST_FILTER    Go test -run regex (default: Federation)
#   TEST_TIMEOUT   Go test timeout (default: 90m)

set -euo pipefail

COMPLEMENT_SRC="${1:?Path to Complement source is required}"
RESULTS_DIR="${2:?Directory for test results is required}"

DIRECTION="${DIRECTION:-both}"
PALPO_IMAGE="${PALPO_IMAGE:-complement-palpo}"
SYNAPSE_IMAGE="${SYNAPSE_IMAGE:-complement-synapse}"
TEST_FILTER="${TEST_FILTER:-Federation}"
TEST_TIMEOUT="${TEST_TIMEOUT:-90m}"

test_packages=(
    ./tests
    ./tests/csapi
)

mkdir -p "$RESULTS_DIR"

run_direction() {
    local name="$1"
    local default_image="$2"
    local hs1_image="$3"
    local hs2_image="$4"
    local dir="$RESULTS_DIR/$name"

    mkdir -p "$dir"

    echo "=== Running mixed federation: $name ==="
    echo "Default image: $default_image"
    echo "HS1 image:     $hs1_image"
    echo "HS2 image:     $hs2_image"

    set +o pipefail
    env -C "$COMPLEMENT_SRC" \
        COMPLEMENT_BASE_IMAGE="$default_image" \
        COMPLEMENT_BASE_IMAGE_HS1="$hs1_image" \
        COMPLEMENT_BASE_IMAGE_HS2="$hs2_image" \
        COMPLEMENT_ENABLE_DIRTY_RUNS=1 \
        COMPLEMENT_SHARE_ENV_PREFIX=PASS_ \
        PASS_SYNAPSE_COMPLEMENT_DATABASE=sqlite \
        go test -tags="palpo_blacklist" -count=1 -timeout "$TEST_TIMEOUT" -run "$TEST_FILTER" -json "${test_packages[@]}" \
        | tee "$dir/results.jsonl"
    local status=${PIPESTATUS[0]}
    set -o pipefail

    grep '^{' "$dir/results.jsonl" \
        | jq -c 'select(.Test != null and (.Action == "pass" or .Action == "fail" or .Action == "skip")) | {Action: .Action, Test: .Test}' \
        | jq -sc 'sort_by(
            (if .Action == "fail" then 0 elif .Action == "skip" then 1 else 2 end),
            .Test
        )[]' > "$dir/__test_all.result.jsonl"

    grep '^{' "$dir/results.jsonl" \
        | jq -r 'select(.Output != null) | .Output' > "$dir/results.log"

    echo "=== Test Summary ($name) ==="
    echo "Total: $(wc -l < "$dir/__test_all.result.jsonl")"
    echo "Pass:  $(grep -c '"pass"' "$dir/__test_all.result.jsonl" || true)"
    echo "Fail:  $(grep -c '"fail"' "$dir/__test_all.result.jsonl" || true)"
    echo "Skip:  $(grep -c '"skip"' "$dir/__test_all.result.jsonl" || true)"

    return "$status"
}

exit_code=0

case "$DIRECTION" in
    synapse-palpo)
        run_direction "synapse-palpo" "$SYNAPSE_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_IMAGE" || exit_code=1
        ;;
    palpo-synapse)
        run_direction "palpo-synapse" "$PALPO_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_IMAGE" || exit_code=1
        ;;
    both)
        run_direction "synapse-palpo" "$SYNAPSE_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_IMAGE" || exit_code=1
        run_direction "palpo-synapse" "$PALPO_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_IMAGE" || exit_code=1
        ;;
    *)
        echo "Unknown DIRECTION: $DIRECTION" >&2
        exit 2
        ;;
esac

exit "$exit_code"
