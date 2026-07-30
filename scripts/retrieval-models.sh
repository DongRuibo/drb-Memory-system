#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

python_bin="${RETRIEVAL_MODELS_PYTHON:-python3}"
export HF_HUB_DISABLE_XET="${HF_HUB_DISABLE_XET:-1}"
host="${RETRIEVAL_MODELS_HOST:-127.0.0.1}"
port="${RETRIEVAL_MODELS_PORT:-18082}"
embedding_model="${MEMORY_EMBEDDING_MODEL:-Qwen/Qwen3-Embedding-0.6B}"
embedding_revision="${MEMORY_EMBEDDING_REVISION:-97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3}"
embedding_dimensions="${MEMORY_EMBEDDING_DIMENSIONS:-1024}"
reranker_model="${MEMORY_DEDICATED_RERANK_MODEL:-Qwen/Qwen3-Reranker-0.6B}"
reranker_revision="${MEMORY_DEDICATED_RERANK_REVISION:-e61197e}"

exec "$python_bin" tools/retrieval_models_server.py \
  --host "$host" \
  --port "$port" \
  --embedding-model "$embedding_model" \
  --embedding-revision "$embedding_revision" \
  --embedding-dimensions "$embedding_dimensions" \
  --reranker-model "$reranker_model" \
  --reranker-revision "$reranker_revision"
