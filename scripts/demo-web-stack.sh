#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
memory_base_url="${MEMORY_BASE_URL:-http://localhost:8080}"
memory_pid=""
web_pid=""

terminate_tree() {
  local target_pid="$1"
  local child_pid
  for child_pid in $(pgrep -P "$target_pid" 2>/dev/null || true); do
    terminate_tree "$child_pid"
  done
  kill "$target_pid" 2>/dev/null || true
}

cleanup() {
  if [ -n "$web_pid" ]; then
    terminate_tree "$web_pid"
  fi
  if [ -n "$memory_pid" ]; then
    terminate_tree "$memory_pid"
  fi
}
trap cleanup EXIT INT TERM

repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
if [ -z "${KIMI_API_KEY:-}" ] \
  && ! grep -Eq '^(KIMI_API_KEY|apikey)=' "${repository_root}/.env" 2>/dev/null; then
  printf 'KIMI_API_KEY is required for extraction and streamed answers.\n' >&2
  exit 1
fi
for required_port in 8080 8090 3000; do
  if lsof -nP -iTCP:"$required_port" -sTCP:LISTEN >/dev/null 2>&1; then
    printf 'Port %s is already in use.\n' "$required_port" >&2
    printf 'Stop the existing local demo process so all services can share one fresh JWT configuration.\n' >&2
    exit 1
  fi
done
if ! command -v openssl >/dev/null 2>&1; then
  printf 'openssl is required to generate the ephemeral local JWT secret.\n' >&2
  exit 1
fi

export MEMORY_AUTH_MODE="jwt_hs256"
export MEMORY_JWT_HS256_SECRET="${MEMORY_JWT_HS256_SECRET:-$(openssl rand -hex 32)}"
export MEMORY_JWT_ISSUER="${MEMORY_JWT_ISSUER:-lazytt-demo-web-api}"
export MEMORY_JWT_AUDIENCE="${MEMORY_JWT_AUDIENCE:-lazytt-memory-service}"
export MEMORY_TENANT_IDS="${MEMORY_TENANT_IDS:-demo-tenant}"
export DEMO_TENANT_ID="${DEMO_TENANT_ID:-demo-tenant}"

"${script_dir}/run-memory-service-semantic.sh" &
memory_pid="$!"

memory_ready="false"
for _ in $(seq 1 300); do
  if curl --fail --silent --max-time 1 "${memory_base_url}/health/ready" >/dev/null 2>&1; then
    memory_ready="true"
    break
  fi
  if ! kill -0 "$memory_pid" 2>/dev/null; then
    wait "$memory_pid"
  fi
  sleep 0.2
done
if [ "$memory_ready" != "true" ]; then
  printf 'Memory Service did not become ready at %s.\n' "$memory_base_url" >&2
  exit 1
fi

"${script_dir}/demo-web.sh" &
web_pid="$!"
wait "$web_pid"
