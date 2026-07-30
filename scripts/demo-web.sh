#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"
web_api_url="${DEMO_WEB_API_URL:-http://127.0.0.1:8090}"
api_pid=""
web_pid=""

export DEMO_USER_ID="${DEMO_USER_ID:-semantic-demo}"
export DEMO_PROJECT_ID="${DEMO_PROJECT_ID:-lazytt-memory-system-demo}"

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
if ! curl --fail --silent --max-time 2 "${web_api_url}/api/health" >/dev/null 2>&1; then
  if command -v cargo >/dev/null 2>&1; then
    cargo run -p demo-web-api &
  elif [ -x "${repository_root}/target/debug/demo-web-api" ]; then
    "${repository_root}/target/debug/demo-web-api" &
  else
    printf 'cargo is unavailable and target/debug/demo-web-api is missing.\n' >&2
    exit 1
  fi
  api_pid="$!"

  api_ready="false"
  for _ in $(seq 1 30); do
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

printf '\nLazyTT Memory Web Demo is ready.\n'
printf 'Open: http://localhost:3000\n'
printf 'Press Ctrl-C here after the meeting.\n\n'

(
  cd "${repository_root}/apps/demo-web"
  exec npm run dev
) &
web_pid="$!"
wait "$web_pid"
