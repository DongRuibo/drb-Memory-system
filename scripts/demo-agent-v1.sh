#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"
dotenv_file="${DEMO_AGENT_ENV_FILE:-${repository_root}/.env}"

if [ -z "${KIMI_API_KEY:-}" ]; then
  dotenv_has_key="false"
  if [ -f "$dotenv_file" ] && [ ! -L "$dotenv_file" ]; then
    if awk -F= '
      /^[[:space:]]*(export[[:space:]]+)?(KIMI_API_KEY|apikey)[[:space:]]*=/ {
        value=substr($0,index($0,"=")+1)
        gsub(/^[[:space:]]+|[[:space:]]+$/,"",value)
        if (length(value)>0) found=1
      }
      END { exit(found ? 0 : 1) }
    ' "$dotenv_file"; then
      dotenv_has_key="true"
    fi
  fi
  if [ "$dotenv_has_key" != "true" ]; then
    printf 'No non-empty KIMI_API_KEY environment variable or apikey entry was found in %s.\n' "$dotenv_file" >&2
    printf 'See docs/demo-agent-v1.md for secure local configuration.\n' >&2
    exit 1
  fi
fi

if ! command -v curl >/dev/null 2>&1; then
  printf 'curl is required for the Memory Service readiness check.\n' >&2
  exit 1
fi

printf '\n[0/1] Check Memory Service readiness\n'
if ! readiness_error="$(curl --fail --silent --show-error \
  "${memory_base_url}/health/ready" 2>&1 >/dev/null)"; then
  printf 'Memory Service is not ready at %s.\n' "$memory_base_url" >&2
  printf '%s\n' "$readiness_error" >&2
  printf 'Start PostgreSQL, Mem0, the Embedder, and memory-service as documented in docs/demo-agent-v1.md.\n' >&2
  exit 1
fi
printf 'ready: %s\n' "$memory_base_url"

export MEMORY_BASE_URL="$memory_base_url"

printf '\n[1/1] Run Demo Agent v1\n'
if command -v cargo >/dev/null 2>&1; then
  exec cargo run -p demo-agent
fi

if [ -x "${repository_root}/target/debug/demo-agent" ]; then
  exec "${repository_root}/target/debug/demo-agent"
fi

printf 'cargo is not in PATH and target/debug/demo-agent does not exist or is not executable.\n' >&2
printf 'Build the demo with cargo build -p demo-agent, then run this script again.\n' >&2
exit 1
