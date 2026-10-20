#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
export DEMO_WEB_API_BIND="${DEMO_WEB_API_BIND:-127.0.0.1:8090}"
memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"
web_api_url="${DEMO_WEB_API_URL:-http://${DEMO_WEB_API_BIND}}"
web_port="${DEMO_WEB_PORT:-3000}"
export NEXT_PUBLIC_DEMO_API_URL="$web_api_url"
export DEMO_WEB_ALLOWED_ORIGINS="${DEMO_WEB_ALLOWED_ORIGINS:-http://localhost:${web_port},http://127.0.0.1:${web_port}}"
api_pid=""
web_pid=""

if [ -z "${MEMORY_JWT_HS256_SECRET:-}" ]; then
  printf 'MEMORY_JWT_HS256_SECRET is required for the multi-user Demo Web API.\n' >&2
  printf 'Use ./scripts/demo-web-stack.sh for the recommended one-command startup.\n' >&2
  exit 1
fi

cleanup() {
  if [ -n "$web_pid" ]; then
    kill "$web_pid" 2>/dev/null || true
  fi
  if [ -n "$api_pid" ]; then
    kill "$api_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

if ! curl --fail --silent --show-error --max-time 3 \
  "${memory_base_url}/health/ready" >/dev/null; then
  printf 'Memory Service is not ready at %s.\n' "$memory_base_url" >&2
  printf 'Start the Demo v0 dependencies and Memory Service first.\n' >&2
  exit 1
fi

if [ ! -d "${repository_root}/apps/demo-web/node_modules" ]; then
  printf 'Web dependencies are missing. Run npm ci in apps/demo-web first.\n' >&2
  exit 1
fi

cd "$repository_root"
if curl --fail --silent --max-time 2 "${web_api_url}/api/health" >/dev/null 2>&1; then
  if ! curl --fail --silent --max-time 5 \
    "${web_api_url}/api/demo/bootstrap" >/dev/null 2>&1; then
    printf 'An incompatible Demo Web API is already listening at %s.\n' "$web_api_url" >&2
    exit 1
  fi
else
  if command -v cargo >/dev/null 2>&1; then
    cargo run -p demo-web-api &
  elif [ -x "${repository_root}/target/debug/demo-web-api" ]; then
    "${repository_root}/target/debug/demo-web-api" &
  else
    printf 'cargo is unavailable and target/debug/demo-web-api is missing. Install Rust with cargo on PATH, or build the binary first.\n' >&2
    exit 1
  fi
  api_pid="$!"

  api_ready="false"
  for _ in $(seq 1 150); do
    if curl --fail --silent --max-time 1 "${web_api_url}/api/health" >/dev/null 2>&1; then
      api_ready="true"
      break
    fi
    sleep 0.2
  done
  if [ "$api_ready" != "true" ]; then
    printf 'Demo Web API did not become ready at %s.\n' "$web_api_url" >&2
    exit 1
  fi
fi

if lsof -nP -iTCP:"$web_port" -sTCP:LISTEN >/dev/null 2>&1; then
  printf 'Port %s is already in use; stop the existing Web process first.\n' "$web_port" >&2
  exit 1
fi

printf '\nLazyTT Memory Web Demo is ready.\n'
printf 'Open: http://localhost:%s\n' "$web_port"
printf 'Press Ctrl-C here after the meeting.\n\n'

(
  cd "${repository_root}/apps/demo-web"
  exec npm run dev -- --hostname 127.0.0.1 --port "$web_port"
) &
web_pid="$!"
wait "$web_pid"
