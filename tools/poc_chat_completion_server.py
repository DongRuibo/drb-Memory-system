#!/usr/bin/env python3
"""Deterministic OpenAI-compatible Chat Completions server for local E2E tests.

This is not a language model. It proves the Demo Agent's HTTP orchestration
without external data egress or credentials. Logs retain only hashes, counts,
and synthetic request IDs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


MAX_BODY_BYTES = 512 * 1024


class ChatCompletionHandler(BaseHTTPRequestHandler):
    server_version = "LazyTTPOCChatCompletion/1.0"

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path == "/health":
            self._json_response(200, {"status": "ok", "model": False})
            return
        self._json_response(404, {"error": {"code": "not_found"}})

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if self.path != "/coding/v1/chat/completions":
            self._json_response(404, {"error": {"code": "not_found"}})
            return

        authorization = self.headers.get("authorization", "")
        if not authorization.startswith("Bearer ") or len(authorization) <= len("Bearer "):
            self._json_response(401, {"error": {"code": "invalid_authentication"}})
            return

        try:
            content_length = int(self.headers.get("content-length", "0"))
            if content_length <= 0 or content_length > MAX_BODY_BYTES:
                raise ValueError(f"body must contain 1 to {MAX_BODY_BYTES} bytes")
            payload = json.loads(self.rfile.read(content_length))
            if payload.get("model") != "k3":
                raise ValueError("model must be k3")
            messages = payload["messages"]
            if not isinstance(messages, list) or len(messages) != 2:
                raise ValueError("messages must contain system and user entries")
            user_payload = messages[1]["content"]
            agent_input = json.loads(user_payload)
            question = agent_input["question"]
            memory_context = agent_input["memory_context"]
            if not isinstance(question, str) or not isinstance(memory_context, list):
                raise ValueError("agent input contract is invalid")
        except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            self._json_response(400, {"error": {"code": "invalid_request"}})
            print(
                json.dumps(
                    {"event": "invalid_request", "error_type": type(error).__name__},
                    separators=(",", ":"),
                ),
                flush=True,
            )
            return

        has_memory = bool(memory_context)
        if has_memory:
            answer = (
                "【原理】\n"
                "事务型 Outbox 像餐厅把“订单已收”和“待通知厨房”写在同一本账上："
                "业务提交成功时，待发送事件也一起持久化，后台再可靠转发。\n\n"
                "【示例】\n"
                "订单事务同时写入订单表和 outbox；Worker 发送失败可重试，"
                "消费者再用幂等键去重。"
            )
        else:
            answer = (
                "事务型 Outbox 在同一个数据库事务中写入业务数据和待发布事件，"
                "后台 Worker 随后读取并发送。这样数据库提交成功时不会遗漏待发送记录，"
                "发送失败也可以继续重试。"
            )

        request_id = uuid.uuid4().hex[:12]
        print(
            json.dumps(
                {
                    "request_id": request_id,
                    "model": payload["model"],
                    "reasoning_effort": payload.get("reasoning_effort"),
                    "memory_context_count": len(memory_context),
                    "user_payload_sha256": hashlib.sha256(
                        user_payload.encode("utf-8")
                    ).hexdigest(),
                },
                separators=(",", ":"),
            ),
            flush=True,
        )
        self._json_response(
            200,
            {
                "id": f"chatcmpl-{request_id}",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": "k3",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "reasoning_content": "deterministic local POC",
                            "content": answer,
                        },
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 0,
                    "completion_tokens": 0,
                    "total_tokens": 0,
                },
            },
            extra_headers={"x-request-id": request_id},
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
    parser.add_argument("--port", type=int, default=18081)
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.host, args.port), ChatCompletionHandler)
    print(
        json.dumps(
            {
                "status": "started",
                "host": args.host,
                "port": args.port,
                "model": False,
                "started_at_unix": int(time.time()),
            },
            separators=(",", ":"),
        ),
        flush=True,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
