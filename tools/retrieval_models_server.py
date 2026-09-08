#!/usr/bin/env python3
"""Local semantic Embedding and Rerank gateway for LazyTT Memory.

The implementation follows the Qwen3-Embedding reference pooling and Qwen3-
Reranker yes/no scoring contracts. Model imports and downloads happen only when
the process starts. Request logs contain hashes and sizes, never source text.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


MAX_BODY_BYTES = 256 * 1024
MAX_INPUTS = 32
MAX_TEXT_CHARS = 32_768
DEFAULT_INSTRUCTION = (
    "Given an agent memory query, retrieve user memories that are relevant "
    "to completing the current task"
)


def _sha256(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _bounded_text(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value.strip() or len(value) > MAX_TEXT_CHARS:
        raise ValueError(f"{name} must contain 1 to {MAX_TEXT_CHARS} characters")
    return value


class RetrievalModels:
    def __init__(self, args: argparse.Namespace) -> None:
        try:
            import torch
            import torch.nn.functional as functional
            from transformers import AutoModel, AutoModelForCausalLM, AutoTokenizer
        except ImportError as error:
            raise RuntimeError(
                "install deploy/models/requirements.txt before starting the semantic model gateway"
            ) from error

        self.torch = torch
        self.functional = functional
        self.max_length = args.max_length
        self.embedding_model_id = args.embedding_model
        self.embedding_aliases = set(args.embedding_alias)
        self.embedding_revision = args.embedding_revision
        self.embedding_dimensions = args.embedding_dimensions
        self.reranker_model_id = args.reranker_model
        self.reranker_revision = args.reranker_revision
        self.instruction = args.instruction
        self.lock = threading.Lock()

        if args.device == "auto":
            if torch.cuda.is_available():
                self.device = "cuda"
            elif getattr(torch.backends, "mps", None) is not None and torch.backends.mps.is_available():
                self.device = "mps"
            else:
                self.device = "cpu"
        else:
            self.device = args.device

        self.embedding_tokenizer = None
        self.embedding_model = None
        if not args.disable_embedding:
            self.embedding_tokenizer = AutoTokenizer.from_pretrained(
                self.embedding_model_id,
                revision=self.embedding_revision,
                padding_side="left",
            )
            self.embedding_model = AutoModel.from_pretrained(
                self.embedding_model_id,
                revision=self.embedding_revision,
            ).to(self.device).eval()
            model_dimensions = int(self.embedding_model.config.hidden_size)
            if not 1 <= self.embedding_dimensions <= model_dimensions:
                raise RuntimeError(
                    "embedding dimensions must be within the model hidden size "
                    f"1..{model_dimensions}"
                )

        self.reranker_tokenizer = None
        self.reranker_model = None
        self.prefix_tokens: list[int] = []
        self.suffix_tokens: list[int] = []
        self.true_token_id = -1
        self.false_token_id = -1
        if not args.disable_reranker:
            self.reranker_tokenizer = AutoTokenizer.from_pretrained(
                self.reranker_model_id,
                revision=self.reranker_revision,
                padding_side="left",
            )
            self.reranker_model = AutoModelForCausalLM.from_pretrained(
                self.reranker_model_id,
                revision=self.reranker_revision,
            ).to(self.device).eval()
            prefix = (
                '<|im_start|>system\nJudge whether the Document meets the requirements '
                'based on the Query and the Instruct provided. Note that the answer can '
                'only be "yes" or "no".<|im_end|>\n<|im_start|>user\n'
            )
            suffix = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
            self.prefix_tokens = self.reranker_tokenizer.encode(
                prefix, add_special_tokens=False
            )
            self.suffix_tokens = self.reranker_tokenizer.encode(
                suffix, add_special_tokens=False
            )
            self.true_token_id = self.reranker_tokenizer.convert_tokens_to_ids("yes")
            self.false_token_id = self.reranker_tokenizer.convert_tokens_to_ids("no")
            if self.true_token_id < 0 or self.false_token_id < 0:
                raise RuntimeError("reranker tokenizer does not contain yes/no tokens")

    def health(self) -> dict[str, Any]:
        return {
            "status": "ok",
            "semantic": True,
            "device": self.device,
            "embedding": None
            if self.embedding_model is None
            else {
                "model": self.embedding_model_id,
                "revision": self.embedding_revision,
                "dimensions": self.embedding_dimensions,
                "normalization": "l2",
                "aliases": sorted(self.embedding_aliases),
            },
            "reranker": None
            if self.reranker_model is None
            else {
                "model": self.reranker_model_id,
                "revision": self.reranker_revision,
            },
        }

    def embed(self, texts: list[str], input_type: str, instruction: str | None) -> list[list[float]]:
        if self.embedding_model is None or self.embedding_tokenizer is None:
            raise ValueError("embedding model is disabled")
        if input_type not in {"query", "document"}:
            raise ValueError("input_type must be query or document")
        effective_instruction = instruction or self.instruction
        model_inputs = (
            [f"Instruct: {effective_instruction}\nQuery:{text}" for text in texts]
            if input_type == "query"
            else texts
        )
        with self.lock, self.torch.no_grad():
            batch = self.embedding_tokenizer(
                model_inputs,
                padding=True,
                truncation=True,
                max_length=self.max_length,
                return_tensors="pt",
            ).to(self.device)
            outputs = self.embedding_model(**batch)
            attention_mask = batch["attention_mask"]
            if attention_mask[:, -1].sum() == attention_mask.shape[0]:
                pooled = outputs.last_hidden_state[:, -1]
            else:
                lengths = attention_mask.sum(dim=1) - 1
                pooled = outputs.last_hidden_state[
                    self.torch.arange(len(model_inputs), device=self.device), lengths
                ]
            pooled = pooled[:, : self.embedding_dimensions]
            pooled = self.functional.normalize(pooled, p=2, dim=1)
            vectors = pooled.float().cpu().tolist()
        for vector in vectors:
            if len(vector) != self.embedding_dimensions or not all(
                math.isfinite(value) for value in vector
            ):
                raise RuntimeError("embedding model returned an invalid vector")
        return vectors

    def rerank(self, query: str, documents: list[str], instruction: str | None) -> list[float]:
        if self.reranker_model is None or self.reranker_tokenizer is None:
            raise ValueError("reranker model is disabled")
        effective_instruction = instruction or self.instruction
        pairs = [
            f"<Instruct>: {effective_instruction}\n<Query>: {query}\n<Document>: {document}"
            for document in documents
        ]
        max_pair_length = self.max_length - len(self.prefix_tokens) - len(self.suffix_tokens)
        tokenized = self.reranker_tokenizer(
            pairs,
            padding=False,
            truncation="longest_first",
            max_length=max_pair_length,
            return_attention_mask=False,
        )
        tokenized["input_ids"] = [
            self.prefix_tokens + item + self.suffix_tokens
            for item in tokenized["input_ids"]
        ]
        batch = self.reranker_tokenizer.pad(
            tokenized,
            padding=True,
            return_tensors="pt",
        ).to(self.device)
        with self.lock, self.torch.no_grad():
            logits = self.reranker_model(**batch).logits[:, -1, :]
            yes_logits = logits[:, self.true_token_id]
            no_logits = logits[:, self.false_token_id]
            scores = self.torch.stack([no_logits, yes_logits], dim=1)
            scores = self.functional.log_softmax(scores, dim=1)[:, 1].exp()
            values = scores.float().cpu().tolist()
        if not all(math.isfinite(value) and 0.0 <= value <= 1.0 for value in values):
            raise RuntimeError("reranker model returned an invalid score")
        return values


class RetrievalHandler(BaseHTTPRequestHandler):
    server_version = "LazyTTRetrievalModels/1.0"
    runtime: RetrievalModels

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/health":
            self._json_response(200, self.runtime.health())
            return
        self._json_response(404, {"error": {"code": "not_found"}})

    def do_POST(self) -> None:  # noqa: N802
        request_id = uuid.uuid4().hex[:16]
        try:
            payload = self._payload()
            if self.path == "/v1/embeddings":
                body, log = self._embeddings(payload)
            elif self.path == "/v1/rerank":
                body, log = self._rerank(payload)
            else:
                self._json_response(404, {"error": {"code": "not_found"}})
                return
        except ValueError as error:
            self._json_response(
                400,
                {"error": {"code": "invalid_request", "message": str(error)}},
                extra_headers={"x-request-id": request_id},
            )
            return
        except Exception as error:  # model failures must not echo source text
            print(
                json.dumps(
                    {
                        "request_id": request_id,
                        "status": "failed",
                        "error_type": type(error).__name__,
                    },
                    separators=(",", ":"),
                ),
                flush=True,
            )
            self._json_response(
                500,
                {"error": {"code": "model_failure", "message": "model inference failed"}},
                extra_headers={"x-request-id": request_id},
            )
            return
        print(
            json.dumps(
                {"request_id": request_id, "status": "ok", **log},
                separators=(",", ":"),
            ),
            flush=True,
        )
        self._json_response(200, body, extra_headers={"x-request-id": request_id})

    def _payload(self) -> dict[str, Any]:
        content_length = int(self.headers.get("content-length", "0"))
        if not 1 <= content_length <= MAX_BODY_BYTES:
            raise ValueError(f"request body must contain 1 to {MAX_BODY_BYTES} bytes")
        try:
            payload = json.loads(self.rfile.read(content_length))
        except json.JSONDecodeError as error:
            raise ValueError("request body must be valid JSON") from error
        if not isinstance(payload, dict):
            raise ValueError("request body must be a JSON object")
        return payload

    def _embeddings(self, payload: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        requested_model = payload.get("model")
        if requested_model != self.runtime.embedding_model_id and requested_model not in self.runtime.embedding_aliases:
            raise ValueError("model does not match the configured embedding model")
        raw_input = payload.get("input")
        inputs = raw_input if isinstance(raw_input, list) else [raw_input]
        if not 1 <= len(inputs) <= MAX_INPUTS:
            raise ValueError(f"input must contain 1 to {MAX_INPUTS} texts")
        texts = [_bounded_text(value, f"input[{index}]") for index, value in enumerate(inputs)]
        requested_dimensions = int(payload.get("dimensions", self.runtime.embedding_dimensions))
        if requested_dimensions != self.runtime.embedding_dimensions:
            raise ValueError("dimensions does not match the configured projection manifest")
        input_type = payload.get("input_type", "document")
        instruction = payload.get("instruction")
        if instruction is not None:
            instruction = _bounded_text(instruction, "instruction")
        vectors = self.runtime.embed(texts, input_type, instruction)
        return (
            {
                "object": "list",
                "data": [
                    {"object": "embedding", "index": index, "embedding": vector}
                    for index, vector in enumerate(vectors)
                ],
                "model": requested_model,
                "usage": {"prompt_tokens": 0, "total_tokens": 0},
            },
            {
                "operation": "embedding",
                "model": self.runtime.embedding_model_id,
                "requested_model": requested_model,
                "input_type": input_type,
                "input_count": len(texts),
                "input_sha256": [_sha256(value) for value in texts],
            },
        )

    def _rerank(self, payload: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
        if payload.get("model") != self.runtime.reranker_model_id:
            raise ValueError("model does not match the configured reranker model")
        query = _bounded_text(payload.get("query"), "query")
        documents = payload.get("documents")
        if not isinstance(documents, list) or not 1 <= len(documents) <= MAX_INPUTS:
            raise ValueError(f"documents must contain 1 to {MAX_INPUTS} items")
        texts = [_bounded_text(value, f"documents[{index}]") for index, value in enumerate(documents)]
        top_n = int(payload.get("top_n", len(texts)))
        if not 1 <= top_n <= len(texts):
            raise ValueError("top_n must be between 1 and the document count")
        instruction = payload.get("instruction")
        if instruction is not None:
            instruction = _bounded_text(instruction, "instruction")
        scores = self.runtime.rerank(query, texts, instruction)
        ranked = sorted(enumerate(scores), key=lambda item: (-item[1], item[0]))[:top_n]
        return (
            {
                "model": self.runtime.reranker_model_id,
                "results": [
                    {"index": index, "relevance_score": score} for index, score in ranked
                ],
            },
            {
                "operation": "rerank",
                "model": self.runtime.reranker_model_id,
                "query_sha256": _sha256(query),
                "document_count": len(texts),
                "document_sha256": [_sha256(value) for value in texts],
            },
        )

    def log_message(self, format_string: str, *args: Any) -> None:
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
    parser.add_argument("--port", type=int, default=18082)
    parser.add_argument("--device", default="auto")
    parser.add_argument("--max-length", type=int, default=4096)
    parser.add_argument("--embedding-model", default="Qwen/Qwen3-Embedding-0.6B")
    parser.add_argument(
        "--embedding-alias",
        action="append",
        default=["text-embedding-3-small"],
        help="additional provider-facing model name routed to the configured embedding model",
    )
    parser.add_argument(
        "--embedding-revision",
        default="97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
    )
    parser.add_argument("--embedding-dimensions", type=int, default=1024)
    parser.add_argument("--reranker-model", default="Qwen/Qwen3-Reranker-0.6B")
    parser.add_argument("--reranker-revision", default="e61197e")
    parser.add_argument("--instruction", default=DEFAULT_INSTRUCTION)
    parser.add_argument("--disable-embedding", action="store_true")
    parser.add_argument("--disable-reranker", action="store_true")
    args = parser.parse_args()
    if not 128 <= args.max_length <= 32_768:
        parser.error("--max-length must be between 128 and 32768")
    if args.disable_embedding and args.disable_reranker:
        parser.error("at least one model must remain enabled")

    RetrievalHandler.runtime = RetrievalModels(args)
    server = ThreadingHTTPServer((args.host, args.port), RetrievalHandler)
    print(
        json.dumps(
            {
                "status": "started",
                "host": args.host,
                "port": args.port,
                "started_at_unix": int(time.time()),
                **RetrievalHandler.runtime.health(),
            },
            separators=(",", ":"),
        ),
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
