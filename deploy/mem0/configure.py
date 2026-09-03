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


def provider_base_url(name: str) -> str:
    raw = required(name, 2048).rstrip("/")
    parsed = urllib.parse.urlparse(raw)
    allowed_http_hosts = {
        "retrieval-models-cpu",
        "retrieval-models-cuda",
        "host.docker.internal",
        "gateway.docker.internal",
    }
    if parsed.scheme == "http" and parsed.hostname not in allowed_http_hosts:
        raise RuntimeError(f"plain HTTP {name} is not an allowed local model endpoint")
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise RuntimeError(f"{name} must be an HTTP(S) URL with a host")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise RuntimeError(f"{name} must not contain credentials, query, or fragment")
    return raw if raw.endswith("/v1") else f"{raw}/v1"


def boolean(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    if raw == "true":
        return True
    if raw == "false":
        return False
    raise RuntimeError(f"{name} must be true or false")


def vector_store_config(dimensions: int) -> dict:
    provider = os.environ.get("MEM0_VECTOR_STORE_PROVIDER", "pgvector").strip()
    if provider == "pgvector":
        return {
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
        }
    if provider != "elasticsearch":
        raise RuntimeError("MEM0_VECTOR_STORE_PROVIDER must be pgvector or elasticsearch")

    host = required("MEM0_ELASTICSEARCH_HOST", 2048).rstrip("/")
    parsed = urllib.parse.urlparse(host)
    if (
        parsed.scheme not in {"http", "https"}
        or not parsed.hostname
        or parsed.username
        or parsed.password
        or parsed.query
        or parsed.fragment
        or parsed.path not in {"", "/"}
    ):
        raise RuntimeError(
            "MEM0_ELASTICSEARCH_HOST must be an HTTP(S) origin without credentials or a path"
        )
    port = int(required("MEM0_ELASTICSEARCH_PORT", 5))
    if not 1 <= port <= 65535:
        raise RuntimeError("MEM0_ELASTICSEARCH_PORT must be between 1 and 65535")
    return {
        "provider": "elasticsearch",
        "config": {
            "host": host,
            "port": port,
            "user": required("MEM0_ELASTICSEARCH_USER", 255),
            "password": required("MEM0_ELASTICSEARCH_PASSWORD"),
            "collection_name": required("MEM0_COLLECTION_NAME", 255),
            "embedding_model_dims": dimensions,
            "verify_certs": boolean("MEM0_ELASTICSEARCH_VERIFY_CERTS", True),
            "use_ssl": parsed.scheme == "https",
            "auto_create_index": True,
        },
    }


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
        "vector_store": vector_store_config(dimensions),
        "embedder": {
            "provider": "openai",
            "config": {
                "api_key": model_api_key,
                "model": required("EMBEDDING_MODEL", 256),
                "embedding_dims": dimensions,
                "openai_base_url": provider_base_url("EMBEDDING_BASE_URL"),
            },
        },
    }
    llm_values = [os.environ.get(name, "").strip() for name in ("LLM_API_KEY", "LLM_BASE_URL", "LLM_MODEL")]
    if any(llm_values):
        if not all(llm_values):
            raise RuntimeError("LLM_API_KEY, LLM_BASE_URL, and LLM_MODEL must be configured together")
        payload["llm"] = {
            "provider": "openai",
            "config": {
                "api_key": required("LLM_API_KEY"),
                "model": required("LLM_MODEL", 256),
                "openai_base_url": provider_base_url("LLM_BASE_URL"),
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
    actual_provider = configured.get("vector_store", {}).get("provider")
    actual_llm_model = configured.get("llm", {}).get("config", {}).get("model")
    expected_llm_model = payload.get("llm", {}).get("config", {}).get("model")
    if (
        actual_dimensions != dimensions
        or actual_collection != payload["vector_store"]["config"]["collection_name"]
        or actual_provider != payload["vector_store"]["provider"]
        or (expected_llm_model is not None and actual_llm_model != expected_llm_model)
    ):
        raise RuntimeError("Mem0 configuration response did not match the requested projection")
    print(
        json.dumps(
            {
                "status": "configured",
                "embedding_model": payload["embedder"]["config"]["model"],
                "embedding_dimensions": dimensions,
                "collection_name": actual_collection,
                "vector_store_provider": actual_provider,
                "llm_model": actual_llm_model,
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
