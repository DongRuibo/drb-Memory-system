#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
export MEMORY_BIND="${MEMORY_BIND:-127.0.0.1:8080}"
export MEMORY_BASE_URL="${MEMORY_BASE_URL:-http://${MEMORY_BIND}}"
export DEMO_WEB_API_BIND="${DEMO_WEB_API_BIND:-127.0.0.1:8090}"
export DEMO_WEB_API_URL="${DEMO_WEB_API_URL:-http://${DEMO_WEB_API_BIND}}"
export DEMO_WEB_PORT="${DEMO_WEB_PORT:-3000}"
memory_base_url="$MEMORY_BASE_URL"
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
if [ -z "${DEMO_CHAT_API_KEY:-${KIMI_API_KEY:-}}" ] \
  && ! grep -Eq '^(DEMO_CHAT_API_KEY|KIMI_API_KEY|apikey)=' "${repository_root}/.env" 2>/dev/null; then
  printf 'DEMO_CHAT_API_KEY (or legacy KIMI_API_KEY) is required for streamed answers; configure extraction separately.\n' >&2
  exit 1
fi
for bind in "$MEMORY_BIND" "$DEMO_WEB_API_BIND" "127.0.0.1:${DEMO_WEB_PORT}"; do
  if [[ "$bind" != 127.0.0.1:* ]]; then
    printf 'The local demo must bind to 127.0.0.1.\n' >&2
    exit 1
  fi
  required_port="${bind##*:}"
  if ! [[ "$required_port" =~ ^[0-9]{1,5}$ ]] \
    || (( 10#$required_port < 1 || 10#$required_port > 65535 )); then
    printf 'Demo ports must be integers from 1 to 65535.\n' >&2
    exit 1
  fi
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
