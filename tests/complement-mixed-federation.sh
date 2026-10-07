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
#   TEST_FILTER    Go test -run regex. Defaults to mixed two-homeserver
#                  federation/interoperability tests.
#   TEST_SKIP      Go test -skip regex (default: known unstable mixed restart
#                  subtests)
#   SYNAPSE_PALPO_TEST_SKIP
#                  Additional go test -skip regex for Synapse -> Palpo
#                  direction. Defaults to the Synapse retry-backoff-sensitive
#                  interrupted to-device subtest.
#   PALPO_SYNAPSE_TEST_SKIP
#                  Additional go test -skip regex for Palpo -> Synapse direction.
#   TEST_TIMEOUT   Go test timeout (default: 90m)
#   ALLOWED_SKIPS  Newline-separated exact subtest names accepted by the results
#                  gate. Defaults only to the documented to-device exclusions.
# Requires Go, Docker, Python 3 and a Complement checkout at the revision used
# by .github/workflows/complement.yml.

set -euo pipefail

COMPLEMENT_SRC="${1:?Path to Complement source is required}"
RESULTS_DIR="${2:?Directory for test results is required}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CASES="$SCRIPT_DIR/complement/mixed-cases.txt"

DIRECTION="${DIRECTION:-both}"
PALPO_IMAGE="${PALPO_IMAGE:-complement-palpo}"
SYNAPSE_IMAGE="${SYNAPSE_IMAGE:-complement-synapse}"
DEFAULT_TEST_FILTER="^($(awk '!/^#/ && NF == 2 {print $2}' "$CASES" | paste -sd '|' -))$"
TEST_FILTER="${TEST_FILTER:-$DEFAULT_TEST_FILTER}"
TEST_SKIP="${TEST_SKIP:-^TestToDeviceMessagesOverFederation$/^stopped_server$}"
SYNAPSE_PALPO_TEST_SKIP="${SYNAPSE_PALPO_TEST_SKIP-^TestToDeviceMessagesOverFederation$/^interrupted_connectivity$}"
PALPO_SYNAPSE_TEST_SKIP="${PALPO_SYNAPSE_TEST_SKIP-}"
TEST_TIMEOUT="${TEST_TIMEOUT:-90m}"

mapfile -t test_packages < <(awk '!/^#/ && NF == 2 {print "./" $1}' "$CASES" | sort -u)

# Keep Palpo-specific coverage in this repository, rather than relying on
# unmerged upstream Complement changes. Copy only our dedicated package.
mkdir -p "$COMPLEMENT_SRC/tests/palpo_mixed"
cp "$SCRIPT_DIR"/complement/mixed/*_test.go "$COMPLEMENT_SRC/tests/palpo_mixed/"

mkdir -p "$RESULTS_DIR"

combine_skip() {
    local base="$1"
    local extra="$2"

    if [[ -n "$base" && -n "$extra" ]]; then
        # Go splits -skip patterns on slash, so combine same-parent subtests at
        # the subtest component instead of alternating whole slash paths.
        local base_parent="${base%%/*}"
        local base_child="${base#*/}"
        local extra_parent="${extra%%/*}"
        local extra_child="${extra#*/}"
        if [[ "$base" == */* && "$extra" == */* && "$base_parent" == "$extra_parent" ]]; then
            printf '%s/(%s|%s)' "$base_parent" "$base_child" "$extra_child"
            return
        fi
        printf '(%s)|(%s)' "$base" "$extra"
    elif [[ -n "$base" ]]; then
        printf '%s' "$base"
    else
        printf '%s' "$extra"
    fi
}

run_direction() {
    local name="$1"
    local default_image="$2"
    local hs1_image="$3"
    local hs2_image="$4"
    local direction_skip="${5:-}"
    local dir="$RESULTS_DIR/$name"
    local effective_skip

    effective_skip="$(combine_skip "$TEST_SKIP" "$direction_skip")"

    mkdir -p "$dir"
    # Invalidate an earlier run before any operation that could fail.
    printf '125\n' > "$dir/exit-code"
    : > "$dir/results.jsonl"

    # The default run must execute every manifest entry. TEST_FILTER overrides
    # are diagnostic runs, and must not be described as full coverage.
    if [[ "$TEST_FILTER" == "$DEFAULT_TEST_FILTER" ]]; then
        cp "$CASES" "$dir/required-tests.txt"
    else
        : > "$dir/required-tests.txt"
        echo "Diagnostic TEST_FILTER override: full coverage gate disabled"
    fi
    if [[ -v ALLOWED_SKIPS ]]; then
        printf '%s\n' "$ALLOWED_SKIPS" > "$dir/allowed-skips.txt"
    else
        printf '%s\n' 'TestToDeviceMessagesOverFederation/stopped_server' > "$dir/allowed-skips.txt"
        if [[ "$name" == "synapse-palpo" ]]; then
            printf '%s\n' 'TestToDeviceMessagesOverFederation/interrupted_connectivity' >> "$dir/allowed-skips.txt"
        fi
    fi

    echo "=== Running mixed federation: $name ==="
    echo "Default image: $default_image"
    echo "HS1 image:     $hs1_image"
    echo "HS2 image:     $hs2_image"
    echo "Test filter:   $TEST_FILTER"
    echo "Test skip:     ${effective_skip:-<none>}"
    git -C "$COMPLEMENT_SRC" rev-parse HEAD > "$dir/complement-revision" || return 1
    printf 'hs1=%s\nhs2=%s\nfilter=%s\nskip=%s\n' \
        "$hs1_image" "$hs2_image" "$TEST_FILTER" "$effective_skip" > "$dir/run-config.txt"
    docker image inspect --format '{{.Id}}' "$hs1_image" "$hs2_image" > "$dir/image-ids.txt" || return 1

    go_test_args=(-tags="palpo_blacklist" -count=1 -timeout "$TEST_TIMEOUT" -run "$TEST_FILTER")
    if [[ -n "$effective_skip" ]]; then
        go_test_args+=(-skip "$effective_skip")
    fi

    set +o pipefail
    env -C "$COMPLEMENT_SRC" \
        COMPLEMENT_BASE_IMAGE="$default_image" \
        COMPLEMENT_BASE_IMAGE_HS1="$hs1_image" \
        COMPLEMENT_BASE_IMAGE_HS2="$hs2_image" \
        COMPLEMENT_ENABLE_DIRTY_RUNS=1 \
        COMPLEMENT_SHARE_ENV_PREFIX=PASS_ \
        PASS_SYNAPSE_COMPLEMENT_DATABASE=sqlite \
        go test -p=1 -parallel=1 "${go_test_args[@]}" -json "${test_packages[@]}" 2>&1 \
        | tee "$dir/results.jsonl"
    local pipeline_status=("${PIPESTATUS[@]}")
    local status="${pipeline_status[0]}"
    if [[ "${pipeline_status[1]}" -ne 0 ]]; then
        status="${pipeline_status[1]}"
    fi
    set -o pipefail
    printf '%s\n' "$status" > "$dir/exit-code"
    python3 "$SCRIPT_DIR/mixed_federation_results.py" "$RESULTS_DIR" --direction "$name"
}

exit_code=0

case "$DIRECTION" in
    synapse-palpo)
        run_direction "synapse-palpo" "$SYNAPSE_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_PALPO_TEST_SKIP" || exit_code=1
        ;;
    palpo-synapse)
        run_direction "palpo-synapse" "$PALPO_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_SYNAPSE_TEST_SKIP" || exit_code=1
        ;;
    both)
        run_direction "synapse-palpo" "$SYNAPSE_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_PALPO_TEST_SKIP" || exit_code=1
        run_direction "palpo-synapse" "$PALPO_IMAGE" "$PALPO_IMAGE" "$SYNAPSE_IMAGE" "$PALPO_SYNAPSE_TEST_SKIP" || exit_code=1
        ;;
    *)
        echo "Unknown DIRECTION: $DIRECTION" >&2
        exit 2
        ;;
esac

exit "$exit_code"
