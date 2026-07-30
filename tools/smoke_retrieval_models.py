#!/usr/bin/env python3
"""Synthetic smoke test for the real local retrieval model gateway."""

from __future__ import annotations

import argparse
import json
import math
import urllib.request
from typing import Any


EMBEDDING_MODEL = "Qwen/Qwen3-Embedding-0.6B"
EMBEDDING_REVISION = "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3"
RERANKER_MODEL = "Qwen/Qwen3-Reranker-0.6B"
RERANKER_REVISION = "e61197e"


def request_json(url: str, payload: dict[str, Any] | None = None) -> dict[str, Any]:
    body = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"content-type": "application/json"} if body is not None else {},
        method="POST" if body is not None else "GET",
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        if response.status != 200:
            raise RuntimeError(f"model gateway returned HTTP {response.status}")
        raw = response.read(4 * 1024 * 1024 + 1)
    if len(raw) > 4 * 1024 * 1024:
        raise RuntimeError("model gateway response exceeded the smoke-test limit")
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise RuntimeError("model gateway response was not an object")
    return value


def cosine(left: list[float], right: list[float]) -> float:
    return sum(a * b for a, b in zip(left, right, strict=True))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:18082")
    args = parser.parse_args()
    base_url = args.base_url.rstrip("/")

    health = request_json(f"{base_url}/health")
    embedding_health = health.get("embedding")
    reranker_health = health.get("reranker")
    if not isinstance(embedding_health, dict) or not isinstance(reranker_health, dict):
        raise RuntimeError("both retrieval models must be enabled")
    if (
        embedding_health.get("model") != EMBEDDING_MODEL
        or embedding_health.get("revision") != EMBEDDING_REVISION
        or embedding_health.get("dimensions") != 1024
        or embedding_health.get("normalization") != "l2"
        or reranker_health.get("model") != RERANKER_MODEL
        or reranker_health.get("revision") != RERANKER_REVISION
    ):
        raise RuntimeError("health model manifest did not match the pinned smoke profile")

    texts = [
        "回答技术问题时先给结论再解释",
        "先说结论，再说明原因",
        "我喜欢周末去爬山",
    ]
    embedded = request_json(
        f"{base_url}/v1/embeddings",
        {
            "model": EMBEDDING_MODEL,
            "input": texts,
            "dimensions": 1024,
            "input_type": "document",
        },
    )
    rows = embedded.get("data")
    if embedded.get("model") != EMBEDDING_MODEL or not isinstance(rows, list) or len(rows) != 3:
        raise RuntimeError("embedding response metadata was invalid")
    vectors: list[list[float]] = []
    norms: list[float] = []
    for index, row in enumerate(rows):
        vector = row.get("embedding") if isinstance(row, dict) else None
        if not isinstance(vector, list) or len(vector) != 1024:
            raise RuntimeError(f"embedding {index} did not contain 1024 dimensions")
        if not all(isinstance(value, (int, float)) and math.isfinite(value) for value in vector):
            raise RuntimeError(f"embedding {index} contained a non-finite value")
        vector = [float(value) for value in vector]
        norm = math.sqrt(sum(value * value for value in vector))
        if abs(norm - 1.0) > 0.01:
            raise RuntimeError(f"embedding {index} was not L2 normalized: {norm}")
        vectors.append(vector)
        norms.append(norm)
    related_similarity = cosine(vectors[0], vectors[1])
    unrelated_similarity = cosine(vectors[0], vectors[2])
    if related_similarity <= unrelated_similarity:
        raise RuntimeError("semantic embedding did not prefer the related Chinese sentence")

    reranked = request_json(
        f"{base_url}/v1/rerank",
        {
            "model": RERANKER_MODEL,
            "query": "用户希望回答采用什么结构？",
            "documents": texts,
            "top_n": len(texts),
        },
    )
    results = reranked.get("results")
    if reranked.get("model") != RERANKER_MODEL or not isinstance(results, list):
        raise RuntimeError("reranker response metadata was invalid")
    indexes = [row.get("index") for row in results if isinstance(row, dict)]
    scores = [row.get("relevance_score") for row in results if isinstance(row, dict)]
    if len(results) != 3 or set(indexes) != {0, 1, 2} or indexes[0] not in {0, 1} or indexes[-1] != 2:
        raise RuntimeError("reranker did not place the unrelated Chinese candidate last")
    if not all(isinstance(score, (int, float)) and 0.0 <= score <= 1.0 for score in scores):
        raise RuntimeError("reranker returned an invalid score")

    print(
        json.dumps(
            {
                "status": "ok",
                "device": health.get("device"),
                "embedding_model": EMBEDDING_MODEL,
                "embedding_revision": EMBEDDING_REVISION,
                "dimensions": 1024,
                "norms": [round(value, 6) for value in norms],
                "related_similarity": round(related_similarity, 6),
                "unrelated_similarity": round(unrelated_similarity, 6),
                "reranker_model": RERANKER_MODEL,
                "reranker_revision": RERANKER_REVISION,
                "reranked_indexes": indexes,
                "reranked_scores": [round(float(value), 6) for value in scores],
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
