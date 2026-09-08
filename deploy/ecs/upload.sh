#!/usr/bin/env bash
# Runner-side transport. All SSH command arguments are restricted to shell-safe values.
set -euo pipefail
set +x
umask 077
: "${ECS_HOST:?}" "${ECS_USER:?}" "${ECS_PORT:?}" "${ECS_REPO_DIR:?}" "${ECS_ENV_FILE:?}"
: "${ECS_SSH_PRIVATE_KEY:?}" "${ECS_KNOWN_HOSTS:?}" "${GHCR_TOKEN:?}" "${GHCR_USER:?}"
[[ "$ECS_HOST" =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ ]]
[[ "$ECS_USER" =~ ^[a-z_][a-z0-9_-]*$ ]]
[[ "$ECS_PORT" =~ ^[0-9]{1,5}$ ]] && (( 10#$ECS_PORT >= 1 && 10#$ECS_PORT <= 65535 ))
[[ "$ECS_REPO_DIR" =~ ^/[a-zA-Z0-9_/-]+$ ]] && [[ "$ECS_REPO_DIR" != / ]]
if [[ ! "$ECS_ENV_FILE" =~ ^/[a-zA-Z0-9_./-]+$ || "$ECS_ENV_FILE" == / ]]; then exit 2; fi
[[ "$GHCR_USER" =~ ^[a-zA-Z0-9_-]+$ ]]
[[ "$1" == prepare || "$1" == prepare-wait || "$1" == release ]]
wait_seconds="${ECS_MIGRATION_WAIT_SECONDS:-1800}"
[[ "$wait_seconds" =~ ^[0-9]{1,4}$ ]] && (( 10#$wait_seconds >= 30 && 10#$wait_seconds <= 3600 ))
[[ "$2" =~ ^[0-9a-f]{40}$ ]]
for image in "$3" "$4"; do [[ "$image" =~ ^ghcr.io/[a-z0-9_./-]+@sha256:[0-9a-f]{64}$ ]]; done
connection_dir="$(mktemp -d)"
stage="/tmp/lazytt-memory-${2}-${GITHUB_RUN_ID:?}-${GITHUB_RUN_ATTEMPT:?}"
[[ "$stage" =~ ^/tmp/lazytt-memory-[a-zA-Z0-9-]+$ ]]
printf '%s\n' "$ECS_SSH_PRIVATE_KEY" > "$connection_dir/key"
printf '%s\n' "$ECS_KNOWN_HOSTS" > "$connection_dir/known_hosts"
ssh_options=(-i "$connection_dir/key" -o BatchMode=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$connection_dir/known_hosts" -o ConnectTimeout=15)
target="$ECS_USER@$ECS_HOST"
uploaded=false
cleanup() {
  if [[ "$uploaded" == true ]]; then
    ssh "${ssh_options[@]}" -p "$ECS_PORT" "$target" "rm -rf -- $stage" </dev/null >/dev/null 2>&1 || true
  fi
  rm -rf -- "$connection_dir"
}
trap cleanup EXIT
ssh "${ssh_options[@]}" -p "$ECS_PORT" "$target" "umask 077; mkdir $stage" </dev/null
uploaded=true
scp "${ssh_options[@]}" -P "$ECS_PORT" compose.dev.yml deploy/ecs/release.py deploy/ecs/remote.sh deploy/ecs/README.md deploy/ecs/环境变量.md deploy/ecs/.env.dev.example "$target:$stage/" >/dev/null
scp -r "${ssh_options[@]}" -P "$ECS_PORT" deploy/ecs/database "$target:$stage/database" >/dev/null
printf '%s' "$GHCR_TOKEN" | ssh "${ssh_options[@]}" -p "$ECS_PORT" "$target" \
  "bash $stage/remote.sh $1 $ECS_REPO_DIR $2 $3 $4 $GHCR_USER $wait_seconds $ECS_ENV_FILE"
