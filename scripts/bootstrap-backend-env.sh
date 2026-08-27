#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(CDPATH= cd -- "${script_dir}/.." && pwd)"
dotenv_path="${repository_root}/.env"

umask 077
touch "$dotenv_path"
chmod 600 "$dotenv_path"

has_nonempty_key() {
  local key="$1"
  awk -F= -v expected="$key" '$1 == expected && length(substr($0, index($0, "=") + 1)) > 0 { found=1 } END { exit !found }' "$dotenv_path"
}

append_generated() {
  local key="$1"
  local value="$2"
  if has_nonempty_key "$key"; then
    return
  fi
  if grep -q "^${key}=" "$dotenv_path"; then
    local temporary_path
    temporary_path="$(mktemp "${dotenv_path}.tmp.XXXXXX")"
    awk -F= -v expected="$key" -v replacement="$value" '
      $1 == expected { print expected "=" replacement; next }
      { print }
    ' "$dotenv_path" >"$temporary_path"
    mv "$temporary_path" "$dotenv_path"
    chmod 600 "$dotenv_path"
  else
    printf '\n%s=%s\n' "$key" "$value" >>"$dotenv_path"
  fi
  printf 'Added %s to .env\n' "$key"
}

random_hex() {
  openssl rand -hex "$1"
}

append_generated MEMORY_POSTGRES_PASSWORD "$(random_hex 24)"
append_generated POSTGRES_PASSWORD "$(random_hex 24)"
append_generated MEM0_POSTGRES_PASSWORD "$(random_hex 24)"
append_generated MEM0_API_KEY "m0-local-$(random_hex 24)"
append_generated MEM0_JWT_SECRET "$(random_hex 48)"
append_generated MEMORY_JWT_HS256_SECRET "$(random_hex 48)"

printf '.env permissions and generated local secrets are ready. Provider keys were not displayed or changed.\n'
