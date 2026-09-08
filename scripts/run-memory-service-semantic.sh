#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

export MEMORY_EMBEDDING_BASE_URL="${MEMORY_EMBEDDING_BASE_URL:-http://127.0.0.1:18082}"
export MEMORY_EMBEDDING_PROVIDER="${MEMORY_EMBEDDING_PROVIDER:-local-qwen}"
export MEMORY_EMBEDDING_MODEL="${MEMORY_EMBEDDING_MODEL:-Qwen/Qwen3-Embedding-0.6B}"
export MEMORY_EMBEDDING_REVISION="${MEMORY_EMBEDDING_REVISION:-97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3}"
export MEMORY_EMBEDDING_DIMENSIONS="${MEMORY_EMBEDDING_DIMENSIONS:-1024}"
export MEMORY_EMBEDDING_NORMALIZATION="${MEMORY_EMBEDDING_NORMALIZATION:-l2}"
export MEMORY_EMBEDDING_INSTRUCTION_VERSION="${MEMORY_EMBEDDING_INSTRUCTION_VERSION:-mem0-symmetric-document-v1}"
export MEMORY_EMBEDDING_SEMANTIC=true
export MEMORY_RERANK_MODE="${MEMORY_RERANK_MODE:-dedicated}"
export MEMORY_DEDICATED_RERANK_BASE_URL="${MEMORY_DEDICATED_RERANK_BASE_URL:-http://127.0.0.1:18082}"
export MEMORY_DEDICATED_RERANK_MODEL="${MEMORY_DEDICATED_RERANK_MODEL:-Qwen/Qwen3-Reranker-0.6B}"
export MEMORY_DEDICATED_RERANK_REVISION="${MEMORY_DEDICATED_RERANK_REVISION:-e61197e}"
export MEMORY_EXTRACT_MODE="${MEMORY_EXTRACT_MODE:-openai_compatible}"
export DEMO_USER_ID="${DEMO_USER_ID:-semantic-demo}"
export DEMO_PROJECT_ID="${DEMO_PROJECT_ID:-lazytt-memory-system-demo}"

if command -v cargo >/dev/null 2>&1; then
  exec cargo run -p memory-service
fi
if [ -x "${repository_root}/target/debug/memory-service" ]; then
  exec "${repository_root}/target/debug/memory-service"
fi
printf 'cargo is unavailable and target/debug/memory-service does not exist. Install Rust with cargo on PATH, or build the binary first.\n' >&2
exit 1
