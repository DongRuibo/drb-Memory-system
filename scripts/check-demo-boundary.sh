#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"

root_migration="${repository_root}/migrations/0030_demo_chat_standalone_snapshot.sql"
demo_migration="${repository_root}/apps/demo-web-api/migrations/0030_demo_chat_standalone_snapshot.sql"

cmp "${root_migration}" "${demo_migration}"

if rg -n 'memory_core|memory_search|\.\./\.\./migrations' \
  --glob '*.sql' \
  --glob 'store.rs' \
  "${repository_root}/apps/demo-web-api/migrations" \
  "${repository_root}/apps/demo-web-api/src/store.rs"; then
  printf 'Demo persistence still depends on a backend schema or migration path.\n' >&2
  exit 1
fi

if rg -n 'memory-domain|memory_domain' \
  "${repository_root}/apps/demo-agent/Cargo.toml" \
  "${repository_root}/apps/demo-agent/src"; then
  printf 'Demo Agent still imports the internal memory-domain crate directly.\n' >&2
  exit 1
fi

printf 'Demo repository boundary checks passed.\n'
