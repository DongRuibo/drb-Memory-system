#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
cd "$repository_root"

has_nonempty_value() {
  local key="$1"
  awk -F= -v expected="$key" '
    $1 == expected && length(substr($0, index($0, "=") + 1)) > 0 { found=1 }
    END { exit !found }
  ' .env
}

has_usable_secret() {
  local key="$1"
  awk -F= -v expected="$key" '
    $1 == expected {
      value=substr($0, index($0, "=") + 1)
      if (length(value) > 0 && value !~ /^replace-with-/) found=1
    }
    END { exit !found }
  ' .env
}

profile="${1:-}"
action="${2:-up}"

case "$profile" in
  cpu|cuda|host-mps|cloud) ;;
  *)
    printf 'Usage: %s <cpu|cuda|host-mps|cloud> [up|down|logs|ps]\n' "$0" >&2
    exit 2
    ;;
esac

case "$action" in
  up)
    ./scripts/bootstrap-backend-env.sh
    extract_mode="$(awk -F= '$1 == "MEMORY_EXTRACT_MODE" { print substr($0, index($0, "=") + 1); exit }' .env)"
    if [ "${extract_mode:-kimi}" = "kimi" ]; then
      if ! has_usable_secret KIMI_EXTRACT_API_KEY \
        && ! has_usable_secret KIMI_API_KEY \
        && ! has_usable_secret apikey; then
        printf 'A non-placeholder Kimi API key is required when MEMORY_EXTRACT_MODE=kimi.\n' >&2
        exit 1
      fi
    fi
    if [ "$profile" = "host-mps" ]; then
      curl --fail --silent --show-error http://127.0.0.1:18082/health >/dev/null || {
        printf 'host-mps requires ./scripts/retrieval-models.sh to be running on port 18082.\n' >&2
        exit 1
      }
    fi
    if [ "$profile" = "cloud" ]; then
      for key in \
        MEMORY_EMBEDDING_PROVIDER MEMORY_EMBEDDING_BASE_URL MEMORY_EMBEDDING_API_KEY \
        MEMORY_EMBEDDING_MODEL MEMORY_EMBEDDING_REVISION MEMORY_EMBEDDING_DIMENSIONS \
        MEMORY_DEDICATED_RERANK_BASE_URL MEMORY_DEDICATED_RERANK_API_KEY \
        MEMORY_DEDICATED_RERANK_MODEL MEMORY_DEDICATED_RERANK_REVISION
      do
        has_nonempty_value "$key" || {
          printf '%s must be set in .env for the cloud profile.\n' "$key" >&2
          exit 1
        }
      done
      for key in MEMORY_EMBEDDING_API_KEY MEMORY_DEDICATED_RERANK_API_KEY
      do
        has_usable_secret "$key" || {
          printf '%s must contain a non-placeholder value for the cloud profile.\n' "$key" >&2
          exit 1
        }
      done
    fi
    running_services="$(docker compose \
      --profile cpu \
      --profile cuda \
      --profile host-mps \
      --profile cloud \
      ps --services --status running)"
    for service in $running_services
    do
      case "$service" in
        memory-service-*)
          if [ "$service" != "memory-service-${profile}" ]; then
            printf 'Backend profile %s is already running. Run %s %s down before switching profiles.\n' \
              "${service#memory-service-}" "$0" "$profile" >&2
            exit 1
          fi
          ;;
      esac
    done
    docker compose --profile "$profile" up --build -d "memory-service-${profile}"
    ;;
  down)
    docker compose \
      --profile cpu \
      --profile cuda \
      --profile host-mps \
      --profile cloud \
      down
    ;;
  logs)
    docker compose --profile "$profile" logs --follow "memory-service-${profile}"
    ;;
  ps)
    docker compose --profile "$profile" ps
    ;;
  *)
    printf 'Action must be up, down, logs, or ps.\n' >&2
    exit 2
    ;;
esac
