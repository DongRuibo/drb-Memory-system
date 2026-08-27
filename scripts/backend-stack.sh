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

config_value() {
  local key="$1"
  local environment_value="${!key-}"
  if [ -n "$environment_value" ]; then
    printf '%s' "$environment_value"
    return
  fi
  if [ -f .env ]; then
    awk -F= -v expected="$key" '
      $1 == expected { print substr($0, index($0, "=") + 1); exit }
    ' .env
  fi
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
  up|down|logs|ps) ;;
  *)
    printf 'Action must be up, down, logs, or ps.\n' >&2
    exit 2
    ;;
esac

if [ "$action" = "up" ]; then
  ./scripts/bootstrap-backend-env.sh
fi

postgres_mode="$(config_value MEMORY_POSTGRES_MODE)"
postgres_mode="${postgres_mode:-local}"
compose_command=(docker compose -f compose.yaml)
case "$postgres_mode" in
  local) ;;
  external)
    compose_command+=(-f compose.external-postgres.yaml)
    if [ "$action" = "up" ]; then
      external_database_url="$(config_value MEMORY_DATABASE_URL)"
      if [ -z "$external_database_url" ]; then
        printf 'MEMORY_DATABASE_URL is required when MEMORY_POSTGRES_MODE=external.\n' >&2
        exit 1
      fi
      case "$external_database_url" in
        postgres://*|postgresql://*) ;;
        *)
          printf 'MEMORY_DATABASE_URL must use the postgres:// or postgresql:// scheme.\n' >&2
          exit 1
          ;;
      esac
      if [[ "$external_database_url" =~ [[:space:][:cntrl:]] ]]; then
        printf 'MEMORY_DATABASE_URL must not contain whitespace or control characters.\n' >&2
        exit 1
      fi
    fi
    ;;
  *)
    printf 'MEMORY_POSTGRES_MODE must be local or external.\n' >&2
    exit 1
    ;;
esac

case "$action" in
  up)
    extract_mode="$(awk -F= '$1 == "MEMORY_EXTRACT_MODE" { print substr($0, index($0, "=") + 1); exit }' .env)"
    extract_mode="${extract_mode:-openai_compatible}"
    case "$extract_mode" in
      openai_compatible)
        if ! has_usable_secret MEMORY_EXTRACT_API_KEY \
          && ! has_usable_secret OPENAI_API_KEY \
          && ! has_usable_secret apikey; then
          printf 'A non-placeholder API key is required when MEMORY_EXTRACT_MODE=openai_compatible.\n' >&2
          exit 1
        fi
        if ! has_nonempty_value MEMORY_EXTRACT_BASE_URL \
          && ! has_nonempty_value OPENAI_BASE_URL \
          && ! has_nonempty_value base_url; then
          printf 'A provider base URL is required when MEMORY_EXTRACT_MODE=openai_compatible.\n' >&2
          exit 1
        fi
        if ! has_nonempty_value MEMORY_EXTRACT_MODEL \
          && ! has_nonempty_value OPENAI_MODEL \
          && ! has_nonempty_value model_name; then
          printf 'A provider model is required when MEMORY_EXTRACT_MODE=openai_compatible.\n' >&2
          exit 1
        fi
        ;;
      kimi)
        if ! has_usable_secret KIMI_EXTRACT_API_KEY \
          && ! has_usable_secret KIMI_API_KEY \
          && ! has_usable_secret apikey; then
          printf 'A non-placeholder Kimi API key is required when MEMORY_EXTRACT_MODE=kimi.\n' >&2
          exit 1
        fi
        ;;
      disabled) ;;
      *)
        printf 'MEMORY_EXTRACT_MODE must be disabled, kimi, or openai_compatible.\n' >&2
        exit 1
        ;;
    esac
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
    running_services="$("${compose_command[@]}" \
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
    printf 'Authoritative PostgreSQL mode: %s.\n' "$postgres_mode"
    "${compose_command[@]}" --profile "$profile" up --build -d "memory-service-${profile}"
    ;;
  down)
    "${compose_command[@]}" \
      --profile cpu \
      --profile cuda \
      --profile host-mps \
      --profile cloud \
      down
    ;;
  logs)
    "${compose_command[@]}" --profile "$profile" logs --follow "memory-service-${profile}"
    ;;
  ps)
    "${compose_command[@]}" --profile "$profile" ps
    ;;
esac
