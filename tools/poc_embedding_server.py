#!/usr/bin/env python3
"""Local deterministic OpenAI-compatible embedding endpoint for contract POCs.

This server is deliberately not a semantic model. It lets the Mem0 POC verify
transport, persistence, filtering, and deletion without external data egress or
provider credentials. Logs contain hashes and sizes, never input text.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


DEFAULT_DIMENSIONS = 1536


def deterministic_embedding(text: str, dimensions: int) -> list[float]:
    """Return a stable non-zero vector; it has no semantic-quality claim."""
    # A content-independent vector makes every stored item eligible for Mem0's
    # score threshold. This POC then tests filters rather than fake similarity.
    del text
    vector = [0.0] * dimensions
    vector[0] = 1.0
    return vector


class EmbeddingHandler(BaseHTTPRequestHandler):
    server_version = "LazyTTPOCEmbedder/1.0"
    default_dimensions = DEFAULT_DIMENSIONS

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path == "/health":
            self._json_response(
                200,
                {
                    "status": "ok",
                    "semantic": False,
                    "dimensions": self.default_dimensions,
                },
            )
            return
        self._json_response(404, {"error": "not_found"})

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path != "/v1/embeddings":
            self._json_response(404, {"error": "not_found"})
            return

        try:
            content_length = int(self.headers.get("content-length", "0"))
            payload = json.loads(self.rfile.read(content_length))
            raw_input = payload["input"]
            inputs = raw_input if isinstance(raw_input, list) else [raw_input]
            if not all(isinstance(value, str) for value in inputs):
                raise ValueError("input must be a string or list of strings")
            dimensions = int(payload.get("dimensions", self.default_dimensions))
            if dimensions <= 0 or dimensions > 4096:
                raise ValueError("dimensions must be between 1 and 4096")
        except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            self._json_response(400, {"error": {"message": str(error)}})
            return

        request_id = uuid.uuid4().hex[:12]
        data = [
            {
                "object": "embedding",
                "index": index,
                "embedding": deterministic_embedding(value, dimensions),
            }
            for index, value in enumerate(inputs)
        ]
        hashes = [hashlib.sha256(value.encode("utf-8")).hexdigest() for value in inputs]
        print(
            json.dumps(
                {
                    "request_id": request_id,
                    "model": payload.get("model"),
                    "dimensions": dimensions,
                    "input_sha256": hashes,
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
        self._json_response(
            200,
            {
                "object": "list",
                "data": data,
                "model": payload.get("model", "poc-deterministic-embedding"),
                "usage": {"prompt_tokens": 0, "total_tokens": 0},
            },
            extra_headers={"x-request-id": request_id},
        )

    def log_message(self, format_string: str, *args: Any) -> None:
        # Avoid BaseHTTPRequestHandler's raw request logging. The structured
        # hash-only event above is sufficient for this POC.
        del format_string, args

    def _json_response(
        self,
        status: int,
        body: dict[str, Any],
        *,
        extra_headers: dict[str, str] | None = None,
    ) -> None:
        encoded = json.dumps(body, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        for key, value in (extra_headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(encoded)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18080)
    parser.add_argument("--dimensions", type=int, default=DEFAULT_DIMENSIONS)
    args = parser.parse_args()
    if args.dimensions <= 0 or args.dimensions > 4096:
        parser.error("--dimensions must be between 1 and 4096")

    EmbeddingHandler.default_dimensions = args.dimensions
    server = ThreadingHTTPServer((args.host, args.port), EmbeddingHandler)
    print(
        json.dumps(
            {
                "status": "started",
                "host": args.host,
                "port": args.port,
                "dimensions": args.dimensions,
                "semantic": False,
                "started_at_unix": int(time.time()),
            },
            separators=(",", ":"),
        ),
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
