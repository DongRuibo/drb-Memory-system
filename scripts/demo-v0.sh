#!/usr/bin/env bash
set -euo pipefail

demo_base_url="${MEMORY_BASE_URL:-http://127.0.0.1:8080}"
demo_run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
remember_key="demo-remember-${demo_run_id}"
forget_key="demo-forget-${demo_run_id}"
expected_memory="我喜欢先讲原理再讲代码"

command -v curl >/dev/null
command -v jq >/dev/null

printf '\n[0/7] Check Memory Service readiness\n'
if ! readiness_error="$(curl --fail --silent --show-error \
  "${demo_base_url}/health/ready" 2>&1 >/dev/null)"; then
  printf 'Memory Service is not ready at %s.\n' "$demo_base_url" >&2
  printf '%s\n' "$readiness_error" >&2
  printf 'Start PostgreSQL, Mem0, the Embedder, and memory-service as documented in docs/demo-v0.md.\n' >&2
  exit 1
fi
printf 'ready: %s\n' "$demo_base_url"

wait_for_projection() {
  operation_id="$1"
  expected_status="$2"
  attempt=0
  while [ "$attempt" -lt 80 ]; do
    response="$(curl --fail --silent --show-error \
      "${demo_base_url}/v1/memory/operations/${operation_id}")"
    actual_status="$(printf '%s' "$response" | jq -r '.projection_status')"
    if [ "$actual_status" = "$expected_status" ]; then
      printf '%s\n' "$response"
      return 0
    fi
    if [ "$actual_status" = "failed" ]; then
      printf 'operation failed: %s\n' "$response" >&2
      return 1
    fi
    sleep 0.25
    attempt=$((attempt + 1))
  done
  printf 'timed out waiting for operation %s to become %s\n' \
    "$operation_id" "$expected_status" >&2
  return 1
}

printf '\n[1/7] Submit an explicit memory event\n'
event_payload="$(jq -n \
  --arg event_id "event-${demo_run_id}" \
  --arg source_ref "conversation:demo-${demo_run_id}:message-1" \
  --arg content "请记住：${expected_memory}" \
  '{
    event_id: $event_id,
    source_type: "conversation_message",
    source_ref: $source_ref,
    content: $content,
    occurred_at: (now | todateiso8601),
    scope_selector: {kind: "user", id: "alice"},
    metadata: {demo: true}
  }')"
remember_response="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -H "idempotency-key: ${remember_key}" \
  -d "$event_payload" \
  "${demo_base_url}/v1/memory/events")"
printf '%s\n' "$remember_response" | jq .
operation_id="$(printf '%s' "$remember_response" | jq -r '.operation_id')"
memory_record_id="$(printf '%s' "$remember_response" | jq -r '.memory_record_id')"
head_revision="$(printf '%s' "$remember_response" | jq -r '.head_revision')"

printf '\n[2/7] Repeat the same request and prove canonical idempotency\n'
repeat_response="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -H "idempotency-key: ${remember_key}" \
  -d "$event_payload" \
  "${demo_base_url}/v1/memory/events")"
repeat_operation_id="$(printf '%s' "$repeat_response" | jq -r '.operation_id')"
test "$repeat_operation_id" = "$operation_id"
printf 'same operation_id: %s\n' "$repeat_operation_id"

printf '\n[3/7] Wait for the Outbox Worker and Mem0 projection\n'
wait_for_projection "$operation_id" ready | jq .

search_payload='{
  "task_context": {
    "task_id": "30000000-0000-0000-0000-000000000001",
    "attempt_id": "30000000-0000-0000-0000-000000000002",
    "purpose": "answer_user"
  },
  "query": "回答技术问题时应该采用什么顺序？",
  "scope_selectors": [{"kind": "user", "id": "alice"}],
  "memory_types": ["preference"],
  "limit": 5
}'

printf '\n[4/7] Search through Mem0 and return PostgreSQL-authorized content\n'
search_response="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -d "$search_payload" \
  "${demo_base_url}/v1/memory/search")"
printf '%s\n' "$search_response" | jq .
test "$(printf '%s' "$search_response" | jq -r '.hits | length')" -eq 1
test "$(printf '%s' "$search_response" | jq -r '.hits[0].content')" = "$expected_memory"
retrieval_run_id="$(printf '%s' "$search_response" | jq -r '.retrieval_run_id')"
test -n "$retrieval_run_id"
test "$retrieval_run_id" != "null"

printf '\n[4a/7] Explain the same retrieval without persisting raw query text\n'
explain_response="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -d "$search_payload" \
  "${demo_base_url}/v1/memory/retrieval:explain")"
printf '%s\n' "$explain_response" | jq .
explain_run_id="$(printf '%s' "$explain_response" | jq -r '.run.retrieval_run_id')"
test "$(printf '%s' "$explain_response" | jq -r '.search.retrieval_run_id')" = "$explain_run_id"
test "$(printf '%s' "$explain_response" | jq -r '.run.query_hash | startswith("sha256:")')" = "true"
test "$(printf '%s' "$explain_response" | jq -r '.run.candidates | length')" -ge 1

persisted_run="$(curl --fail --silent --show-error \
  "${demo_base_url}/v1/memory/retrieval-runs/${explain_run_id}")"
test "$(printf '%s' "$persisted_run" | jq -r '.retrieval_run_id')" = "$explain_run_id"

printf '\n[5/7] Commit canonical Forget\n'
forget_payload="$(jq -n \
  --argjson expected_revision "$head_revision" \
  '{expected_revision: $expected_revision, reason: "Demo user requested forget"}')"
forget_response="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -H "idempotency-key: ${forget_key}" \
  -d "$forget_payload" \
  "${demo_base_url}/v1/memories/${memory_record_id}/forget")"
printf '%s\n' "$forget_response" | jq .
forget_operation_id="$(printf '%s' "$forget_response" | jq -r '.operation_id')"

printf '\n[6/7] Prove immediate authority denial before relying on Provider deletion\n'
after_forget="$(curl --fail --silent --show-error \
  -H 'content-type: application/json' \
  -d "$search_payload" \
  "${demo_base_url}/v1/memory/search")"
printf '%s\n' "$after_forget" | jq .
test "$(printf '%s' "$after_forget" | jq -r '.hits | length')" -eq 0

printf '\n[7/7] Wait for physical projection deletion\n'
wait_for_projection "$forget_operation_id" deleted | jq .

printf '\nDemo v0 PASSED: remember -> canonical commit -> outbox -> Mem0 -> authority read -> forget.\n'
