#!/usr/bin/env python3
"""Bounded SiliconFlow Embedding/Reranker smoke without logging API keys."""

from __future__ import annotations

import json
import os
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


def load_dotenv(path: Path) -> dict[str, str]:
    mode = path.stat().st_mode
    if mode & (stat.S_IRWXG | stat.S_IRWXO):
        raise RuntimeError(".env must not be accessible by group or others")
    values: dict[str, str] = {}
    for line_number, raw in enumerate(path.read_text().splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise RuntimeError(f".env line {line_number} must use NAME=value")
        key, value = line.split("=", 1)
        if not key.replace("_", "a").isalnum() or not key[0].isalpha():
            raise RuntimeError(f".env line {line_number} has an invalid name")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        if key in values:
            raise RuntimeError(f".env defines {key} more than once")
        values[key] = value
    return values


def required(values: dict[str, str], name: str) -> str:
    value = os.environ.get(name) or values.get(name, "")
    if not value.strip():
        raise RuntimeError(f"{name} is required")
    return value.strip()


def siliconflow_base(values: dict[str, str], name: str) -> str:
    raw = required(values, name).rstrip("/")
    parsed = urllib.parse.urlparse(raw)
    if (
        parsed.scheme != "https"
        or parsed.hostname != "api.siliconflow.cn"
        or parsed.port not in {None, 443}
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or parsed.path not in {"", "/v1"}
    ):
        raise RuntimeError(f"{name} must use the official SiliconFlow HTTPS API endpoint")
    return raw.removesuffix("/v1")


def post_json(url: str, api_key: str, payload: dict) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, separators=(",", ":")).encode(),
        method="POST",
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            data = response.read(1024 * 1024)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"provider returned HTTP {error.code}") from None
    decoded = json.loads(data)
    if not isinstance(decoded, dict):
        raise RuntimeError("provider returned a non-object response")
    return decoded


def main() -> None:
    values = load_dotenv(Path(".env"))
    embedding_base = siliconflow_base(values, "MEMORY_EMBEDDING_BASE_URL")
    embedding_key = required(values, "MEMORY_EMBEDDING_API_KEY")
    embedding_model = required(values, "MEMORY_EMBEDDING_MODEL")
    dimensions = int(required(values, "MEMORY_EMBEDDING_DIMENSIONS"))
    embedding = post_json(
        f"{embedding_base}/v1/embeddings",
        embedding_key,
        {
            "model": embedding_model,
            "input": "LazyTT semantic embedding readiness probe",
            "dimensions": dimensions,
        },
    )
    data = embedding.get("data")
    if embedding.get("model") != embedding_model or not isinstance(data, list) or len(data) != 1:
        raise RuntimeError("embedding response model or vector count did not match")
    vector = data[0].get("embedding") if isinstance(data[0], dict) else None
    if not isinstance(vector, list) or len(vector) != dimensions:
        raise RuntimeError("embedding response dimensions did not match")

    rerank_base = siliconflow_base(values, "MEMORY_DEDICATED_RERANK_BASE_URL")
    rerank_key = required(values, "MEMORY_DEDICATED_RERANK_API_KEY")
    rerank_model = required(values, "MEMORY_DEDICATED_RERANK_MODEL")
    rerank = post_json(
        f"{rerank_base}/v1/rerank",
        rerank_key,
        {
            "model": rerank_model,
            "query": "Which memory is relevant to the current project database?",
            "documents": [
                "The current project uses PostgreSQL as its authority store.",
                "The user watched a movie last week.",
            ],
            "top_n": 2,
            "instruction": "Rank memories by relevance to the current task.",
        },
    )
    results = rerank.get("results")
    if not isinstance(results, list) or len(results) != 2:
        raise RuntimeError("reranker did not return exactly one score per document")
    indexes = {item.get("index") for item in results if isinstance(item, dict)}
    if indexes != {0, 1}:
        raise RuntimeError("reranker response indexes did not match the request")

    print(
        json.dumps(
            {
                "status": "ok",
                "embedding_model": embedding_model,
                "embedding_dimensions": dimensions,
                "reranker_model": rerank_model,
            },
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
                separators=(",", ":"),
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)
