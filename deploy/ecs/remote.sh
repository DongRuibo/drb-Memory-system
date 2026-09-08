#!/usr/bin/env bash
# Runs on ECS from a unique temporary upload directory; token arrives only on stdin.
set -euo pipefail
set +x
umask 077
stage_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
action="$1"; deployment_dir="$2"; revision="$3"; memory_image="$4"; mem0_image="$5"; registry_user="$6"; config_file="${8:?Explicit ECS_ENV_FILE required}"
if [[ ! "$config_file" =~ ^/[a-zA-Z0-9_./-]+$ || "$config_file" == / ]]; then exit 2; fi
registry_config="$(mktemp -d)"
# upload.sh owns the upload directory; never delete the directory containing this script here.
cleanup() { rm -rf -- "$registry_config"; }
trap cleanup EXIT
export DOCKER_CONFIG="$registry_config"
docker login ghcr.io --username "$registry_user" --password-stdin >/dev/null 2>&1
if [[ "$action" == release ]]; then
  python3 "$stage_dir/release.py" release --directory "$deployment_dir" --config "$config_file" --revision "$revision" \
    --memory-image "$memory_image" --mem0-image "$mem0_image" </dev/null
else
  python3 "$stage_dir/release.py" prepare --directory "$deployment_dir" --config "$config_file" \
    --source "$stage_dir/compose.dev.yml" --revision "$revision" \
    --memory-image "$memory_image" --mem0-image "$mem0_image" </dev/null
fi
if [[ "$action" == prepare-wait ]]; then
  deadline=$((SECONDS + ${7:-1800}))
  printf 'Waiting for administrator database migration; candidate %s.\n' "$revision"
  until python3 "$stage_dir/release.py" database-check --directory "$deployment_dir" --config "$config_file" --revision "$revision" \
      --memory-image "$memory_image" --mem0-image "$mem0_image" </dev/null; do
    if (( SECONDS >= deadline )); then
      printf 'Database wait timed out. Candidate and admin bundle remain on ECS; rerun failed deployment job after migration.\n' >&2
      exit 1
    fi
    sleep 15
  done
fi
