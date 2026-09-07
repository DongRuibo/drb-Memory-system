#!/usr/bin/env bash
# Runs on ECS from a unique temporary upload directory; token arrives only on stdin.
set -euo pipefail
set +x
umask 077
stage_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
action="$1"; deployment_dir="$2"; revision="$3"; memory_image="$4"; mem0_image="$5"; registry_user="$6"
registry_config="$(mktemp -d)"
cleanup() { rm -rf -- "$registry_config" "$stage_dir"; }
trap cleanup EXIT
export DOCKER_CONFIG="$registry_config"
docker login ghcr.io --username "$registry_user" --password-stdin >/dev/null 2>&1
python3 "$stage_dir/release.py" prepare --directory "$deployment_dir" \
  --source "$stage_dir/compose.dev.yml" --revision "$revision" \
  --memory-image "$memory_image" --mem0-image "$mem0_image" </dev/null
if [[ "$action" == release ]]; then
  python3 "$stage_dir/release.py" release --directory "$deployment_dir" </dev/null
fi
