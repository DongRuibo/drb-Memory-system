#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

memory_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:18084}"

if ! curl --fail --silent --show-error "${memory_base_url}/health/ready" >/dev/null; then
  printf 'Retrieval evaluation requires a ready isolated Memory Service at %s.\n' "$memory_base_url" >&2
  printf 'Start it with DEMO_TENANT_ID=retrieval-eval-v1 DEMO_USER_ID=evaluator MEMORY_BIND=127.0.0.1:18084.\n' >&2
  exit 1
fi

export MEMORY_BASE_URL="$memory_base_url"
export EVAL_DATASET="${EVAL_DATASET:-evals/retrieval-v1/dataset.json}"
export EVAL_PROVIDER_PROFILE="${EVAL_PROVIDER_PROFILE:-hash-only-poc}"
export EVAL_TENANT_ID="${EVAL_TENANT_ID:-retrieval-eval-v1}"
export EVAL_USER_ID="${EVAL_USER_ID:-evaluator}"

if command -v cargo >/dev/null 2>&1; then
  exec cargo run -p retrieval-eval
fi

if [ -x "${repository_root}/target/debug/retrieval-eval" ]; then
  exec "${repository_root}/target/debug/retrieval-eval"
fi

printf 'cargo is not in PATH and target/debug/retrieval-eval is unavailable.\n' >&2
exit 1
