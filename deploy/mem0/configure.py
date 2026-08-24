#!/usr/bin/env python3
"""Configure the pinned Mem0 projection without printing provider credentials."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


def required(name: str, maximum: int = 4096) -> str:
    value = os.environ.get(name, "").strip()
    if not value or len(value) > maximum or any(ord(char) < 32 for char in value):
        raise RuntimeError(f"{name} must be set to a bounded visible value")
    return value


def provider_base_url() -> str:
    raw = required("EMBEDDING_BASE_URL", 2048).rstrip("/")
    parsed = urllib.parse.urlparse(raw)
    allowed_http_hosts = {
        "retrieval-models-cpu",
        "retrieval-models-cuda",
        "host.docker.internal",
        "gateway.docker.internal",
    }
    if parsed.scheme == "http" and parsed.hostname not in allowed_http_hosts:
        raise RuntimeError("plain HTTP embedding URL is not an allowed local model endpoint")
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise RuntimeError("EMBEDDING_BASE_URL must be an HTTP(S) URL with a host")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise RuntimeError("EMBEDDING_BASE_URL must not contain credentials, query, or fragment")
    return raw if raw.endswith("/v1") else f"{raw}/v1"


def request_json(url: str, method: str, api_key: str, payload: dict | None = None) -> dict:
    body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
    request = urllib.request.Request(
        url,
        data=body,
        method=method,
        headers={
            "Content-Type": "application/json",
            "X-API-Key": api_key,
        },
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        data = response.read(1024 * 1024)
    decoded = json.loads(data)
    if not isinstance(decoded, dict):
        raise RuntimeError("Mem0 returned a non-object response")
    return decoded


def main() -> None:
    mem0_base_url = required("MEM0_BASE_URL", 2048).rstrip("/")
    mem0_api_key = required("MEM0_API_KEY")
    model_api_key = required("EMBEDDING_API_KEY")
    dimensions = int(required("EMBEDDING_DIMENSIONS", 8))
    if not 1 <= dimensions <= 4096:
        raise RuntimeError("EMBEDDING_DIMENSIONS must be between 1 and 4096")

    last_error = "Mem0 did not become ready"
    for _ in range(60):
        try:
            request_json(f"{mem0_base_url}/auth/setup-status", "GET", mem0_api_key)
            break
        except (OSError, ValueError, urllib.error.HTTPError) as error:
            last_error = f"{type(error).__name__}"
            time.sleep(2)
    else:
        raise RuntimeError(last_error)

    payload = {
        "vector_store": {
            "provider": "pgvector",
            "config": {
                "host": required("MEM0_POSTGRES_HOST", 255),
                "port": int(os.environ.get("MEM0_POSTGRES_PORT", "5432")),
                "dbname": required("MEM0_POSTGRES_DB", 255),
                "user": required("MEM0_POSTGRES_USER", 255),
                "password": required("MEM0_POSTGRES_PASSWORD"),
                "collection_name": required("MEM0_COLLECTION_NAME", 255),
                "embedding_model_dims": dimensions,
            },
        },
        "embedder": {
            "provider": "openai",
            "config": {
                "api_key": model_api_key,
                "model": required("EMBEDDING_MODEL", 256),
                "embedding_dims": dimensions,
                "openai_base_url": provider_base_url(),
            },
        },
    }
    request_json(f"{mem0_base_url}/configure", "POST", mem0_api_key, payload)
    configured = request_json(f"{mem0_base_url}/configure", "GET", mem0_api_key)
    actual_dimensions = (
        configured.get("embedder", {}).get("config", {}).get("embedding_dims")
    )
    actual_collection = (
        configured.get("vector_store", {}).get("config", {}).get("collection_name")
    )
    if actual_dimensions != dimensions or actual_collection != payload["vector_store"]["config"]["collection_name"]:
        raise RuntimeError("Mem0 configuration response did not match the requested projection")
    print(
        json.dumps(
            {
                "status": "configured",
                "embedding_model": payload["embedder"]["config"]["model"],
                "embedding_dimensions": dimensions,
                "collection_name": actual_collection,
            },
            separators=(",", ":"),
        ),
        flush=True,
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(
            json.dumps(
                {"status": "failed", "error_type": type(error).__name__, "detail": str(error)},
                separators=(",", ":"),
            ),
            file=sys.stderr,
            flush=True,
        )
        raise SystemExit(1)
