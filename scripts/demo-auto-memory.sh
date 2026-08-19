#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"

if ! command -v curl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  printf 'curl and jq are required.\n' >&2
  exit 1
fi

service_info="$(curl --fail --silent --show-error "${memory_base_url}/")"
if [ "$(jq -r '.extractor // "disabled"' <<<"$service_info")" = "disabled" ]; then
  printf 'Memory Service automatic extraction is disabled; start it with MEMORY_EXTRACT_MODE=openai_compatible or kimi.\n' >&2
  exit 1
fi
if [ "$(jq -r '.projection_model_manifest.semantic_embedding' <<<"$service_info")" != "true" ]; then
  printf 'Memory Service is not using a declared semantic Embedding manifest.\n' >&2
  exit 1
fi

export MEMORY_BASE_URL="$memory_base_url"
export DEMO_AGENT_MEMORY_MODE=automatic
export DEMO_USER_ID="${DEMO_USER_ID:-semantic-demo}"
export MEMORY_TIMEOUT_SECONDS="${MEMORY_TIMEOUT_SECONDS:-30}"
exec "${repository_root}/scripts/demo-agent-v1.sh"
