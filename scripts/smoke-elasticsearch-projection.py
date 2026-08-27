#!/usr/bin/env python3
"""Isolated Elasticsearch 8.11 projection smoke without logging credentials."""

from __future__ import annotations

import json
import os
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


INDEX_NAME = "lazytt-memory-poc-v1"
NAMESPACE = "poc-tenant:poc-user"
GENERATION = "elasticsearch-poc-v1"
PERSISTENCE_ID = "poc-persistence-marker"
MAX_RESPONSE_BYTES = 4 * 1024 * 1024


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: ANN001
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def load_dotenv(path: Path) -> dict[str, str]:
    mode = path.stat().st_mode
    if mode & (stat.S_IRWXG | stat.S_IRWXO):
        raise RuntimeError(".env must not be accessible by group or others; run chmod 600 .env")
    values: dict[str, str] = {}
    for line_number, raw in enumerate(path.read_text().splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise RuntimeError(f".env line {line_number} must use NAME=value")
        key, value = line.split("=", 1)
        if not key or not key.replace("_", "a").isalnum() or not key[0].isalpha():
            raise RuntimeError(f".env line {line_number} has an invalid name")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        if key in values:
            raise RuntimeError(f".env defines {key} more than once")
        values[key] = value
    return values


def value(values: dict[str, str], name: str, default: str = "") -> str:
    return (os.environ.get(name) or values.get(name, default)).strip()


def required(values: dict[str, str], name: str) -> str:
    result = value(values, name)
    if not result:
        raise RuntimeError(f"{name} is required")
    return result


def normalized_base_url(raw: str, *, local_only: bool) -> str:
    parsed = urllib.parse.urlparse(raw.rstrip("/"))
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise RuntimeError("base URL must use HTTP or HTTPS")
    if parsed.username is not None or parsed.password is not None or parsed.query or parsed.fragment:
        raise RuntimeError("base URL must not contain credentials, query parameters or fragments")
    loopback = parsed.hostname in {"127.0.0.1", "localhost", "::1"}
    if local_only and (parsed.scheme != "http" or not loopback):
        raise RuntimeError("Elasticsearch POC URL must be loopback HTTP")
    if not local_only and parsed.scheme != "https" and not loopback:
        raise RuntimeError("remote embedding URL must use HTTPS")
    return raw.rstrip("/")


def request_json(
    method: str,
    url: str,
    payload: dict[str, Any] | None = None,
    *,
    headers: dict[str, str] | None = None,
    expected: tuple[int, ...] = (200,),
    timeout: int = 60,
) -> tuple[int, dict[str, Any]]:
    request_headers = {"Accept": "application/json"}
    if headers:
        request_headers.update(headers)
    data = None
    if payload is not None:
        data = json.dumps(payload, separators=(",", ":")).encode()
        request_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers=request_headers,
    )
    try:
        with OPENER.open(request, timeout=timeout) as response:
            body = response.read(MAX_RESPONSE_BYTES + 1)
            status = response.status
    except urllib.error.HTTPError as error:
        body = error.read(2048)
        if error.code in expected:
            return error.code, json.loads(body) if body else {}
        detail = body.decode(errors="replace").replace("\n", " ")[:512]
        raise RuntimeError(f"{method} request returned HTTP {error.code}: {detail}") from None
    except urllib.error.URLError as error:
        raise RuntimeError(f"{method} request failed: {error.reason}") from None
    if len(body) > MAX_RESPONSE_BYTES:
        raise RuntimeError("response exceeded 4 MiB")
    if status not in expected:
        raise RuntimeError(f"{method} request returned unexpected HTTP {status}")
    if not body:
        return status, {}
    decoded = json.loads(body)
    if not isinstance(decoded, dict):
        raise RuntimeError("endpoint returned a non-object JSON response")
    return status, decoded


def embedding_endpoint(base_url: str) -> str:
    return f"{base_url.removesuffix('/v1')}/v1/embeddings"


def embed(
    base_url: str,
    api_key: str,
    model: str,
    dimensions: int,
    text: str,
    input_type: str,
    dialect: str,
) -> list[float]:
    payload: dict[str, Any] = {
        "model": model,
        "input": text,
        "dimensions": dimensions,
    }
    if dialect == "lazytt":
        payload["input_type"] = input_type
    _, body = request_json(
        "POST",
        embedding_endpoint(base_url),
        payload,
        headers={"Authorization": f"Bearer {api_key}"},
    )
    data = body.get("data")
    if body.get("model") != model or not isinstance(data, list) or len(data) != 1:
        raise RuntimeError("embedding response model or vector count did not match")
    vector = data[0].get("embedding") if isinstance(data[0], dict) else None
    if not isinstance(vector, list) or len(vector) != dimensions:
        raise RuntimeError("embedding response dimensions did not match")
    if not all(isinstance(item, (int, float)) for item in vector):
        raise RuntimeError("embedding response contains a non-numeric value")
    return [float(item) for item in vector]


def ensure_index(es_url: str, dimensions: int) -> None:
    status, _ = request_json(
        "HEAD",
        f"{es_url}/{INDEX_NAME}",
        expected=(200, 404),
    )
    if status == 404:
        request_json(
            "PUT",
            f"{es_url}/{INDEX_NAME}",
            {
                "settings": {"number_of_shards": 1, "number_of_replicas": 0},
                "mappings": {
                    "dynamic": "strict",
                    "properties": {
                        "operation_id": {"type": "keyword"},
                        "namespace_key": {"type": "keyword"},
                        "generation": {"type": "keyword"},
                        "memory_record_id": {"type": "keyword"},
                        "memory_version_id": {"type": "keyword"},
                        "head_revision": {"type": "long"},
                        "content_hash": {"type": "keyword"},
                        "content": {"type": "text"},
                        "embedding": {
                            "type": "dense_vector",
                            "dims": dimensions,
                            "index": True,
                            "similarity": "cosine",
                        },
                    },
                },
            },
        )
    _, mapping = request_json("GET", f"{es_url}/{INDEX_NAME}/_mapping")
    actual = (
        mapping.get(INDEX_NAME, {})
        .get("mappings", {})
        .get("properties", {})
        .get("embedding", {})
        .get("dims")
    )
    if actual != dimensions:
        raise RuntimeError(
            f"existing index uses {actual!r} embedding dimensions; expected {dimensions}"
        )


def make_document(
    operation_id: str,
    content: str,
    vector: list[float],
    *,
    namespace: str = NAMESPACE,
    generation: str = GENERATION,
) -> dict[str, Any]:
    return {
        "operation_id": operation_id,
        "namespace_key": namespace,
        "generation": generation,
        "memory_record_id": f"record-{operation_id}",
        "memory_version_id": f"version-{operation_id}",
        "head_revision": 1,
        "content_hash": f"hash-{operation_id}",
        "content": content,
        "embedding": vector,
    }


def put_document(es_url: str, document_id: str, document: dict[str, Any]) -> dict[str, Any]:
    _, result = request_json(
        "PUT",
        f"{es_url}/{INDEX_NAME}/_doc/{urllib.parse.quote(document_id)}?refresh=wait_for",
        document,
        expected=(200, 201),
    )
    return result


def filtered_hits(body: dict[str, Any]) -> list[dict[str, Any]]:
    hits = body.get("hits", {}).get("hits")
    if not isinstance(hits, list):
        raise RuntimeError("Elasticsearch search response did not contain hits.hits")
    for hit in hits:
        source = hit.get("_source") if isinstance(hit, dict) else None
        if not isinstance(source, dict):
            raise RuntimeError("Elasticsearch hit did not contain _source")
        if source.get("namespace_key") != NAMESPACE or source.get("generation") != GENERATION:
            raise RuntimeError("namespace or generation filter leaked an unauthorized document")
    return hits


def run_smoke(values: dict[str, str]) -> None:
    es_url = normalized_base_url(
        value(values, "ELASTICSEARCH_POC_URL", "http://127.0.0.1:19200"),
        local_only=True,
    )
    embedding_base = normalized_base_url(
        required(values, "MEMORY_EMBEDDING_BASE_URL"),
        local_only=False,
    )
    embedding_key = required(values, "MEMORY_EMBEDDING_API_KEY")
    embedding_model = required(values, "MEMORY_EMBEDDING_MODEL")
    dimensions = int(required(values, "MEMORY_EMBEDDING_DIMENSIONS"))
    dialect = value(values, "MEMORY_EMBEDDING_REQUEST_DIALECT", "openai")
    if dialect not in {"openai", "lazytt"}:
        raise RuntimeError("MEMORY_EMBEDDING_REQUEST_DIALECT must be openai or lazytt")
    if not 1 <= dimensions <= 4096:
        raise RuntimeError("MEMORY_EMBEDDING_DIMENSIONS must be between 1 and 4096")

    _, root = request_json("GET", es_url)
    elasticsearch_version = root.get("version", {}).get("number")
    if elasticsearch_version != "8.11.3":
        raise RuntimeError(
            f"Elasticsearch version must be 8.11.3; received {elasticsearch_version!r}"
        )
    _, health = request_json("GET", f"{es_url}/_cluster/health")
    if health.get("status") not in {"yellow", "green"}:
        raise RuntimeError("Elasticsearch cluster is not ready")
    ensure_index(es_url, dimensions)

    cooking = "用户于2026年7月29日自己做了红烧牛腩。"
    preference = "用户偏好回答技术问题时先给一句结论，再列出要点。"
    query = "我前天自己做了什么饭？"
    cooking_vector = embed(
        embedding_base, embedding_key, embedding_model, dimensions, cooking, "document", dialect
    )
    preference_vector = embed(
        embedding_base, embedding_key, embedding_model, dimensions, preference, "document", dialect
    )
    query_vector = embed(
        embedding_base, embedding_key, embedding_model, dimensions, query, "query", dialect
    )

    primary = make_document("poc-cooking", cooking, cooking_vector)
    first = put_document(es_url, "poc-cooking", primary)
    second = put_document(es_url, "poc-cooking", primary)
    if first.get("result") not in {"created", "updated"} or second.get("result") != "updated":
        raise RuntimeError("deterministic document upsert did not behave idempotently")
    put_document(es_url, "poc-preference", make_document("poc-preference", preference, preference_vector))
    put_document(
        es_url,
        "poc-other-user",
        make_document(
            "poc-other-user",
            cooking,
            cooking_vector,
            namespace="poc-tenant:other-user",
        ),
    )
    put_document(
        es_url,
        "poc-other-generation",
        make_document(
            "poc-other-generation",
            cooking,
            cooking_vector,
            generation="elasticsearch-poc-old",
        ),
    )
    put_document(
        es_url,
        PERSISTENCE_ID,
        make_document(PERSISTENCE_ID, "LazyTT Elasticsearch 持久化标记。", preference_vector),
    )

    _, count = request_json(
        "POST",
        f"{es_url}/{INDEX_NAME}/_count",
        {"query": {"term": {"operation_id": "poc-cooking"}}},
    )
    if count.get("count") != 1:
        raise RuntimeError("repeated upsert created more than one logical document")

    filters = [
        {"term": {"namespace_key": NAMESPACE}},
        {"term": {"generation": GENERATION}},
    ]
    _, vector_search = request_json(
        "POST",
        f"{es_url}/{INDEX_NAME}/_search",
        {
            "size": 3,
            "knn": {
                "field": "embedding",
                "query_vector": query_vector,
                "k": 3,
                "num_candidates": 10,
                "filter": {"bool": {"filter": filters}},
            },
        },
    )
    vector_hits = filtered_hits(vector_search)
    if not vector_hits or vector_hits[0].get("_id") != "poc-cooking":
        raise RuntimeError("vector search did not rank the relevant memory first")

    _, bm25_search = request_json(
        "POST",
        f"{es_url}/{INDEX_NAME}/_search",
        {
            "size": 3,
            "query": {
                "bool": {
                    "must": [{"match": {"content": "红烧牛腩"}}],
                    "filter": filters,
                }
            },
        },
    )
    bm25_hits = filtered_hits(bm25_search)
    if not bm25_hits or bm25_hits[0].get("_id") != "poc-cooking":
        raise RuntimeError("BM25 search did not rank the relevant memory first")

    _, lookup = request_json("GET", f"{es_url}/{INDEX_NAME}/_doc/poc-cooking")
    source = lookup.get("_source", {})
    if source.get("memory_record_id") != "record-poc-cooking":
        raise RuntimeError("exact lookup did not return the expected memory metadata")

    put_document(es_url, "poc-delete", make_document("poc-delete", preference, preference_vector))
    request_json(
        "DELETE",
        f"{es_url}/{INDEX_NAME}/_doc/poc-delete?refresh=wait_for",
        expected=(200,),
    )
    deleted_status, _ = request_json(
        "GET",
        f"{es_url}/{INDEX_NAME}/_doc/poc-delete",
        expected=(404,),
    )
    if deleted_status != 404:
        raise RuntimeError("deleted projection document remained visible")

    print(
        json.dumps(
            {
                "status": "ok",
                "elasticsearch_version": elasticsearch_version,
                "index": INDEX_NAME,
                "embedding_model": embedding_model,
                "embedding_dimensions": dimensions,
                "checks": [
                    "mapping",
                    "idempotent_upsert",
                    "vector_search",
                    "bm25_search",
                    "namespace_generation_filter",
                    "lookup",
                    "delete",
                    "persistence_marker_written",
                ],
            },
            ensure_ascii=False,
            separators=(",", ":"),
        )
    )


def verify_persistence(values: dict[str, str]) -> None:
    es_url = normalized_base_url(
        value(values, "ELASTICSEARCH_POC_URL", "http://127.0.0.1:19200"),
        local_only=True,
    )
    _, body = request_json("GET", f"{es_url}/{INDEX_NAME}/_doc/{PERSISTENCE_ID}")
    if body.get("found") is not True:
        raise RuntimeError("persistence marker was not found after restart")
    print(
        json.dumps(
            {
                "status": "ok",
                "check": "restart_persistence",
                "index": INDEX_NAME,
                "document_id": PERSISTENCE_ID,
            },
            separators=(",", ":"),
        )
    )


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in {"run", "verify-persistence"}:
        raise RuntimeError(
            "usage: scripts/smoke-elasticsearch-projection.py run|verify-persistence"
        )
    values = load_dotenv(Path(".env"))
    if sys.argv[1] == "run":
        run_smoke(values)
    else:
        verify_persistence(values)


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
