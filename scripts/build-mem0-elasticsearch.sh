#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

base_image="lazytt-mem0-api:b357a5a1"
elasticsearch_image="lazytt-mem0-api-elasticsearch:b357a5a1"
mem0_source="https://github.com/mem0ai/mem0.git#b357a5a1b03c299ec8229c268e63cfac0f7c6566"

if ! docker image inspect "$base_image" >/dev/null 2>&1; then
  docker build --file server/dev.Dockerfile --tag "$base_image" "$mem0_source"
fi

docker build \
  --file deploy/mem0/Dockerfile.elasticsearch \
  --tag "$elasticsearch_image" \
  .
