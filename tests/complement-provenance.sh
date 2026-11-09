#!/usr/bin/env bash
# Record source identity, executable/image identity, features and fixture config.
set -euo pipefail
complement_src="${1:?Complement checkout}"
image="${2:?Palpo image}"
output="${3:?Evidence file}"
features="${4:?Feature description}"
synapse_src="${5:-}"
synapse_image="${6:-complement-synapse}"
repo="$(git -C "$(dirname -- "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
mkdir -p "$(dirname -- "$output")"
palpo_revision="$(git -C "$repo" rev-parse HEAD)"
complement_revision="$(git -C "$complement_src" rev-parse HEAD)"
synapse_revision=""
if [[ -n "$synapse_src" ]]; then
    synapse_revision="$(git -C "$synapse_src" rev-parse HEAD)"
fi
{
    printf 'palpo_revision=%s\n' "$palpo_revision"
    printf 'complement_revision=%s\n' "$complement_revision"
    if [[ -n "$synapse_src" ]]; then
        printf 'synapse_revision=%s\n' "$synapse_revision"
        printf 'synapse_image=%s\n' "$synapse_image"
        docker image inspect --format 'synapse_image_id={{.Id}}' "$synapse_image"
    fi
    printf 'palpo_features=%s\n' "$features"
    printf 'palpo_image=%s\n' "$image"
    docker image inspect --format 'palpo_image_id={{.Id}}' "$image"
    docker run --rm --entrypoint sha256sum "$image" /work/palpo
    sha256sum "$repo/tests/complement/palpo.toml" "$repo/tests/complement/caddy.json"
    go version
} > "$output"
cat "$output"
