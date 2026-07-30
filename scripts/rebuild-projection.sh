#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"
source_generation="${SOURCE_GENERATION:-demo-v0-generation-1}"
target_generation="${1:-rebuild-$(date -u +%Y%m%dT%H%M%SZ)}"
switch_action="${REBUILD_SWITCH_ACTION:-finalize}"
poll_limit="${REBUILD_POLL_LIMIT:-300}"
bearer_token="${MEMORY_BEARER_TOKEN:-}"

if ! command -v jq >/dev/null 2>&1; then
  printf 'jq is required for projection rebuild validation.\n' >&2
  exit 1
fi
if [[ ! "$target_generation" =~ ^[A-Za-z0-9._-]{1,96}$ ]]; then
  printf 'Target generation must match ^[A-Za-z0-9._-]{1,96}$ for stable action keys.\n' >&2
  exit 1
fi
if [ "$switch_action" != "finalize" ] && [ "$switch_action" != "rollback" ]; then
  printf 'REBUILD_SWITCH_ACTION must be finalize or rollback.\n' >&2
  exit 1
fi
if ! [[ "$poll_limit" =~ ^[1-9][0-9]*$ ]]; then
  printf 'REBUILD_POLL_LIMIT must be a positive integer.\n' >&2
  exit 1
fi

request() {
  if [ -n "$bearer_token" ]; then
    curl --fail --silent --show-error \
      -H "Authorization: Bearer ${bearer_token}" "$@"
  else
    curl --fail --silent --show-error "$@"
  fi
}

printf '[1/5] Start isolated generation %s from %s\n' "$target_generation" "$source_generation"
request \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: rebuild-start-${target_generation}" \
  -d "$(jq -cn --arg generation "$target_generation" '{generation: $generation}')" \
  "${memory_base_url}/v1/projection-generations/rebuilds" |
  jq .

printf '[2/5] Wait for all canonical heads to become ready\n'
poll=0
while :; do
  status_json="$(request \
    "${memory_base_url}/v1/projection-generations/${target_generation}")"
  status="$(jq -r '.status' <<<"$status_json")"
  ready="$(jq -r '.ready_records' <<<"$status_json")"
  expected="$(jq -r '.expected_records' <<<"$status_json")"
  failed="$(jq -r '.failed_records' <<<"$status_json")"
  printf 'status=%s ready=%s/%s failed=%s\n' "$status" "$ready" "$expected" "$failed"
  if [ "$status" = "ready" ]; then
    break
  fi
  if [ "$status" = "failed" ]; then
    printf 'Projection rebuild failed; inspect generation status before retrying.\n' >&2
    exit 1
  fi
  poll=$((poll + 1))
  if [ "$poll" -ge "$poll_limit" ]; then
    printf 'Projection rebuild did not become ready within the poll limit.\n' >&2
    exit 1
  fi
  sleep 1
done

printf '[3/5] Validate and switch reads to canary generation\n'
request \
  -H 'Content-Type: application/json' \
  -H "Idempotency-Key: rebuild-activate-${target_generation}" \
  -d "$(jq -cn --arg generation "$source_generation" \
    '{expected_active_generation: $generation}')" \
  "${memory_base_url}/v1/projection-generations/${target_generation}/activate" |
  jq .

printf '[4/5] Canary is active; writes remain frozen until finalize or rollback\n'
request "${memory_base_url}/v1/projection-generations/${target_generation}" | jq .

printf '[5/5] %s generation switch\n' "$switch_action"
request \
  -X POST \
  -H "Idempotency-Key: rebuild-${switch_action}-${target_generation}" \
  "${memory_base_url}/v1/projection-generations/${target_generation}/${switch_action}" |
  jq .

printf 'Projection rebuild flow completed with action=%s.\n' "$switch_action"
