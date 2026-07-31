#!/bin/sh
set -eu

exec python /app/retrieval_models_server.py \
  --host "${RETRIEVAL_MODELS_HOST:-0.0.0.0}" \
  --port "${RETRIEVAL_MODELS_PORT:-18082}" \
  --device "${RETRIEVAL_MODELS_DEVICE:-auto}" \
  --max-length "${RETRIEVAL_MODELS_MAX_LENGTH:-4096}" \
  --embedding-model "${MEMORY_EMBEDDING_MODEL:-Qwen/Qwen3-Embedding-0.6B}" \
  --embedding-revision "${MEMORY_EMBEDDING_REVISION:-97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3}" \
  --embedding-dimensions "${MEMORY_EMBEDDING_DIMENSIONS:-1024}" \
  --reranker-model "${MEMORY_DEDICATED_RERANK_MODEL:-Qwen/Qwen3-Reranker-0.6B}" \
  --reranker-revision "${MEMORY_DEDICATED_RERANK_REVISION:-e61197e}"
