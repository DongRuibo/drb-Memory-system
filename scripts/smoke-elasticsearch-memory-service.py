#!/usr/bin/env python3
"""Exercise natural-turn extraction and recall through Elasticsearch."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


SESSION_ID = "39b8f9cb-bd1c-4d9f-b62d-cfaf30a26ee1"
TURN_ID = "62f606b8-139e-4b79-b52c-5a97c8c805f3"
IDEMPOTENCY_KEY = "elasticsearch-runtime-poc-turn-0002"
COMPLETED_AT = "2026-08-21T08:00:00Z"
MEMORY_SENTENCE = "回答技术问题时先给一句结论，再列出要点"


def base_url() -> str:
    raw = os.environ.get("ELASTICSEARCH_MEMORY_URL", "http://127.0.0.1:18081").rstrip("/")
    parsed = urllib.parse.urlparse(raw)
    if (
        parsed.scheme != "http"
        or parsed.hostname not in {"127.0.0.1", "localhost", "::1"}
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise RuntimeError("ELASTICSEARCH_MEMORY_URL must be a loopback HTTP URL")
    return raw


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    *,
    headers: dict[str, str] | None = None,
    expected: tuple[int, ...] = (200,),
) -> dict[str, Any]:
    request_headers = {"Accept": "application/json"}
    if headers:
        request_headers.update(headers)
    data = None
    if payload is not None:
        data = json.dumps(payload, separators=(",", ":")).encode()
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, method=method, headers=request_headers)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = response.read(4 * 1024 * 1024)
            status = response.status
    except urllib.error.HTTPError as error:
        detail = error.read(2048).decode(errors="replace").replace("\n", " ")[:512]
        raise RuntimeError(f"{method} {url} returned HTTP {error.code}: {detail}") from None
    except urllib.error.URLError as error:
        raise RuntimeError(f"{method} {url} failed: {error.reason}") from None
    if status not in expected:
        raise RuntimeError(f"{method} {url} returned unexpected HTTP {status}")
    if not body:
        return {}
    decoded = json.loads(body)
    if not isinstance(decoded, dict):
        raise RuntimeError("Memory Service returned non-object JSON")
    return decoded


def identity_headers() -> dict[str, str]:
    return {
        "X-Tenant-Id": os.environ.get(
            "ELASTICSEARCH_POC_TENANT_ID", "elasticsearch-poc-tenant"
        ),
        "X-User-Id": os.environ.get(
            "ELASTICSEARCH_POC_USER_ID", "elasticsearch-poc-user"
        ),
    }


def main() -> None:
    if len(sys.argv) > 2 or (len(sys.argv) == 2 and sys.argv[1] != "verify-recall"):
        raise RuntimeError(
            "usage: scripts/smoke-elasticsearch-memory-service.py [verify-recall]"
        )
    verify_only = len(sys.argv) == 2
    url = base_url()
    request_json("GET", f"{url}/health/ready")
    root = request_json("GET", url)
    manifest = root.get("projection_model_manifest", {})
    if root.get("projection") != "elasticsearch" or manifest.get("index_provider") != "elasticsearch":
        raise RuntimeError("Memory Service is not running with the Elasticsearch projection")

    headers = identity_headers()
    per_turn: dict[str, Any] = {}
    if not verify_only:
        extract_headers = dict(headers)
        extract_headers["Idempotency-Key"] = IDEMPOTENCY_KEY
        extraction = request_json(
            "POST",
            f"{url}/v1/runtime/memory/extract",
            {
                "session_id": SESSION_ID,
                "turn_id": TURN_ID,
                "user_content": "请记住：我回答技术问题时喜欢先给一句结论，再列出要点。",
                "assistant_content": "知道了，以后回答技术问题时会遵循这个顺序。",
                "completed_at": COMPLETED_AT,
                "timezone": "Asia/Shanghai",
            },
            headers=extract_headers,
            expected=(202,),
        )
        per_turn = extraction.get("completed_turn", {}).get("per_turn_extraction", {})
        if per_turn.get("candidate_count", 0) < 1:
            raise RuntimeError(
                "extractor did not produce a memory candidate: "
                + json.dumps(per_turn.get("diagnostics", {}), ensure_ascii=False)
            )

    retrieved: dict[str, Any] | None = None
    for _ in range(60):
        candidate = request_json(
            "POST",
            f"{url}/v1/runtime/memory/retrieve",
            {
                "query": "我偏好怎样回答技术问题？",
                "memory_types": ["preference"],
                "limit": 10,
                "token_budget": 2048,
                "memory_required": False,
                "timezone": "Asia/Shanghai",
            },
            headers=headers,
        )
        rendered_sections = json.dumps(
            candidate.get("context_pack", {}).get("sections", []),
            ensure_ascii=False,
        )
        if MEMORY_SENTENCE in rendered_sections:
            retrieved = candidate
            break
        time.sleep(1)
    if retrieved is None:
        raise RuntimeError("new memory was not recalled through Elasticsearch within 60 seconds")

    context_pack = retrieved["context_pack"]
    if context_pack.get("consumed_at") is None:
        raise RuntimeError("runtime retrieve did not consume the immutable Context Pack")
    print(
        json.dumps(
            {
                "status": "ok",
                "mode": "verify-recall" if verify_only else "extract-and-recall",
                "projection": "elasticsearch",
                "candidate_count": per_turn.get("candidate_count"),
                "projection_generation": context_pack.get("projection_generation"),
                "context_pack_id": context_pack.get("context_pack_id"),
                "checks": (
                    ["restart_persistence", "runtime_retrieve", "context_pack_consumed"]
                    if verify_only
                    else [
                        "runtime_extract",
                        "postgres_authority_write",
                        "elasticsearch_projection",
                        "runtime_retrieve",
                        "context_pack_consumed",
                    ]
                ),
            },
            ensure_ascii=False,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(
            json.dumps(
                {"status": "failed", "error_type": type(error).__name__, "detail": str(error)},
                ensure_ascii=False,
                separators=(",", ":"),
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)
