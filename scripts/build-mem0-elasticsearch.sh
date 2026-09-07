#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

mem0_revision="b357a5a1b03c299ec8229c268e63cfac0f7c6566"
source_dir="$(mktemp -d)"
trap 'rm -rf -- "$source_dir"' EXIT
curl --fail --silent --show-error --location \
  "https://codeload.github.com/mem0ai/mem0/tar.gz/$mem0_revision" \
  --output "$source_dir/source.tar.gz"
mkdir -p "$source_dir/_upstream/mem0" "$source_dir/deploy/mem0" "$source_dir/deploy/ecs/database"
tar -xzf "$source_dir/source.tar.gz" --strip-components=1 -C "$source_dir/_upstream/mem0"
cp deploy/mem0/Dockerfile deploy/mem0/*.py deploy/mem0/alembic.ini "$source_dir/deploy/mem0/"
cp -R deploy/mem0/migration "$source_dir/deploy/mem0/migration"
cp deploy/ecs/database.py deploy/ecs/configuration.py "$source_dir/deploy/ecs/"
cp deploy/ecs/database/comments.json deploy/ecs/database/03_verify.sql "$source_dir/deploy/ecs/database/"

docker build \
  --file "$source_dir/deploy/mem0/Dockerfile" \
  --tag lazytt-mem0-api-elasticsearch:b357a5a1-es-gateway-v1 \
  "$source_dir"
python3 tests/ecs/check_mem0_source.py lazytt-mem0-api-elasticsearch:b357a5a1-es-gateway-v1 \
  "$source_dir/_upstream/mem0"
