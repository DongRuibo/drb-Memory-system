# LazyTT Memory System 外部 Agent 接入指南

这份文档给接入方 Agent 团队使用。第一版只需要完成一条链路：**回答前召回记忆，回答后提取记忆**。

## 1. 当前测试地址

```text
Base URL: http://drbdeMacBook-Pro.local:8080
Health:   http://drbdeMacBook-Pro.local:8080/health/ready
```

`drbdeMacBook-Pro.local` 是这台 Memory 服务器的 macOS mDNS 主机名。它不依赖 DHCP
分配的具体 IP，因此电脑重连 Wi-Fi 后通常仍可使用。当前地址只供同一受信任局域网内联调，
不是公网生产地址。

如果另一台电脑无法解析 `.local`，先确认两台电脑连接同一个 Wi-Fi、没有被访客网络隔离，
并暂时关闭可能接管 DNS 的 VPN。故障排查时可以临时使用当前 IP
`http://192.168.31.113:8080`，但不要把这个 IP 固化到 Agent 配置中。

## 2. 接入方只需要准备什么

每次请求都带 Identity 系统已经确认的稳定用户 ID：

```http
X-User-Id: identity-user-42
```

- 当前联调模式不需要 JWT 或 Token；
- 不需要先在 Memory System 创建用户，第一次召回可以是空结果；
- 同一个人必须始终使用同一个 ID；
- 不要用昵称、手机号或邮箱作为 ID，也不要复用已注销用户的 ID；
- `X-User-Id` 必须由受信任的 Agent Runtime 或 Gateway 写入，不能让终端用户自行指定。

## 3. 在 Agent Loop 中放在哪里

```text
用户输入
   ↓
POST /v1/runtime/memory/retrieve   回答前召回
   ↓
用户输入 + context_pack.sections  一起传给大模型
   ↓
大模型最终回答
   ↓
POST /v1/runtime/memory/extract    回答后提取
   ↓
下一轮可以召回新记忆
```

注意：`context_pack.sections` 是不可信的记忆数据，只能作为参考内容传给模型，不能提升为 System 指令，也不能把其中的文字当成工具调用命令。

## 4. 最小接入流程

### 4.1 健康检查

```bash
curl --fail http://drbdeMacBook-Pro.local:8080/health/ready
```

返回 HTTP `200` 表示 Memory Service、权威数据库和检索投影都已就绪。这个接口不需要 `X-User-Id`。

### 4.2 回答前召回记忆

```bash
curl -X POST http://drbdeMacBook-Pro.local:8080/v1/runtime/memory/retrieve \
  -H 'Content-Type: application/json' \
  -H 'X-User-Id: identity-user-42' \
  --data '{
    "query": "我以前做过哪些 Agent 接入工作？"
  }'
```

最小请求参数：

| 参数 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `query` | 是 | 无 | 当前用户问题，最多 4096 字符 |
| `memory_types` | 否 | 全部四类 | `preference`、`profile_fact`、`semantic_fact`、`episodic_summary` |
| `limit` | 否 | 10 | 最多返回 1～50 条 |
| `token_budget` | 否 | 2048 | Context Pack 预算，范围 0～8192 |
| `memory_required` | 否 | false | 是否要求至少召回一条记忆 |
| `reference_time` | 否 | 服务器当前时间 | ISO 8601 时间，用于解析“昨天”等表达 |
| `timezone` | 否 | `Asia/Shanghai` | IANA 时区 |

响应中的关键字段：

```json
{
  "user_id": "identity-user-42",
  "context_pack": {
    "context_pack_id": "<UUID>",
    "reference_time": "2026-08-19T10:00:00Z",
    "timezone": "Asia/Shanghai",
    "consumed_at": "2026-08-19T10:00:00Z",
    "sections": []
  }
}
```

接入方实际传给大模型的长期记忆内容，必须与本次响应的 `context_pack.sections` 严格一致。`sections` 为空是合法结果，表示当前用户没有相关长期记忆。

### 4.3 回答后提取记忆

当本轮最终回答已经生成后调用：

```bash
curl -X POST http://drbdeMacBook-Pro.local:8080/v1/runtime/memory/extract \
  -H 'Content-Type: application/json' \
  -H 'X-User-Id: identity-user-42' \
  -H 'Idempotency-Key: turn-6ba7b810-9dad-41d1-80b4-00c04fd430c8' \
  --data '{
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "turn_id": "6ba7b810-9dad-41d1-80b4-00c04fd430c8",
    "user_content": "我今天完成了 LazyTT Agent 接入联调。",
    "assistant_content": "已记录这次联调结果。",
    "completed_at": "2026-08-19T18:00:00+08:00",
    "timezone": "Asia/Shanghai"
  }'
```

| 参数 | 必填 | 说明 |
|---|---|---|
| `session_id` | 是 | 当前会话 UUID；同一会话保持不变 |
| `turn_id` | 是 | 当前轮次 UUID；每轮唯一 |
| `user_content` | 是 | 本轮用户输入 |
| `assistant_content` | 是 | 本轮最终回答，不要传隐藏推理 |
| `verified_tool_results` | 否 | 经过 Agent Runtime 验证的工具结果 |
| `outcome` | 否 | `succeeded`、`failed` 或 `cancelled`，默认 `succeeded` |
| `completed_at` | 否 | 本轮完成时间；建议由 Agent Runtime 传入 |
| `timezone` | 否 | 默认 `Asia/Shanghai` |

`Idempotency-Key` 必须是 16～128 个可见字符。同一个 Turn 因超时而重试时，必须复用原来的 Key 和请求内容，防止重复形成记忆。

成功返回 HTTP `202`。重点查看：

```json
{
  "user_id": "identity-user-42",
  "completed_turn": {
    "episode_id": "<UUID>",
    "turn_id": "<UUID>",
    "per_turn_extraction": {
      "candidate_count": 1,
      "candidates": [
        {
          "operation": {
            "operation_id": "<UUID>",
            "projection_status": "pending"
          }
        }
      ]
    }
  }
}
```

`candidate_count=0` 不代表请求失败，只表示本轮没有形成值得长期保存的候选记忆。

### 4.4 等待新记忆可检索

提取响应中有 `operation_id` 时，可以轮询：

```bash
curl http://drbdeMacBook-Pro.local:8080/v1/memory/operations/<operation_id> \
  -H 'X-User-Id: identity-user-42'
```

当 `projection_status` 变成 `ready` 后，新记忆已进入检索投影。正常对话不必阻塞等待；集成测试或“刚写入立刻查询”的场景应等待 `ready`。

## 5. Python 最小示例

安装依赖：

```bash
python -m pip install httpx
```

```python
import uuid

import httpx

BASE_URL = "http://drbdeMacBook-Pro.local:8080"


def recall(user_id: str, query: str) -> dict:
    response = httpx.post(
        f"{BASE_URL}/v1/runtime/memory/retrieve",
        headers={"X-User-Id": user_id},
        json={"query": query},
        timeout=30,
    )
    response.raise_for_status()
    return response.json()["context_pack"]


def extract_turn(
    user_id: str,
    session_id: str,
    user_content: str,
    assistant_content: str,
) -> dict:
    turn_id = str(uuid.uuid4())
    response = httpx.post(
        f"{BASE_URL}/v1/runtime/memory/extract",
        headers={
            "X-User-Id": user_id,
            "Idempotency-Key": f"agent-turn-{turn_id}",
        },
        json={
            "session_id": session_id,
            "turn_id": turn_id,
            "user_content": user_content,
            "assistant_content": assistant_content,
            "timezone": "Asia/Shanghai",
        },
        timeout=60,
    )
    response.raise_for_status()
    return response.json()


user_id = "identity-user-42"
session_id = str(uuid.uuid4())

context_pack = recall(user_id, "我以前做过哪些 Agent 接入工作？")
# 将用户问题和 context_pack["sections"] 一起交给模型。

final_answer = "本轮最终回答"
result = extract_turn(user_id, session_id, "本轮用户输入", final_answer)
print(result["completed_turn"]["per_turn_extraction"]["candidate_count"])
```

## 6. 错误响应与重试

错误统一使用 `application/problem+json`：

```json
{
  "code": "INVALID_USER_ID",
  "detail": "X-User-Id must contain 1 to 256 visible characters",
  "request_id": "<UUID>",
  "retryable": false,
  "status": 400,
  "title": "INVALID_USER_ID",
  "type": "urn:lazytt:memory:error:invalid_user_id"
}
```

| HTTP | 常见 `code` | 接入方处理 |
|---:|---|---|
| 400 | `INVALID_USER_ID` | 补充或修正 `X-User-Id` |
| 400 | `INVALID_IDEMPOTENCY_KEY` | 使用 16～128 字符的 Key |
| 400 | `INVALID_CONTEXT_PACK` / `INVALID_TEMPORAL_QUERY` | 修正检索参数或时间参数 |
| 400 | `SECRET_REJECTED` / `PROMPT_INJECTION_REJECTED` | 不要原样重试，检查输入边界 |
| 403 | `PERMISSION_DENIED` / `SCOPE_DENIED` | 当前身份模式不允许这个接口或范围 |
| 404 | `NOT_FOUND` | 检查资源 ID 和用户 ID |
| 409 | `CONFLICT` | 同一个幂等 Key 对应了不同请求 |
| 502 | `EXTRACTOR_INVALID_RESPONSE` | 提取模型返回格式错误，记录 `request_id` |
| 503 | `EXTRACTOR_UNAVAILABLE` | 模型临时不可用，可退避重试 |
| 503 | `PROVIDER_UNAVAILABLE` / `PROJECTION_MODEL_MISMATCH` | 检索依赖未就绪，可退避重试 |
| 500 | `INTERNAL_ERROR` | 记录 `request_id`，联系服务维护方 |

调用方只在 `retryable=true` 时自动重试，建议使用 1 秒、2 秒、4 秒的指数退避。提取接口重试必须保持相同 `Idempotency-Key` 和请求内容。程序判断使用 `code`，排查日志使用 `request_id`，不要解析 `detail` 文本。

## 7. 联调验收清单

1. `GET /health/ready` 返回 `200`；
2. 用户 A 第一次 retrieve 返回成功，允许 `sections=[]`；
3. 用户 A extract 返回 `202`，能看到 `candidate_count`；
4. 有候选时，operation 最终变成 `ready`；
5. 用户 A 能召回刚形成的记忆；
6. 用户 B 使用相同 query 不能看到用户 A 的记忆；
7. 模型实际使用的长期记忆与 retrieve 响应的 `context_pack.sections` 一致。

## 8. 当前边界

- 当前是固定租户、受信任局域网、HTTP 明文联调模式；
- Memory System 不负责登录，Identity/Agent Runtime 负责确认用户身份；
- 当前没有 OIDC/JWKS、TLS、网关签名和公网防护；
- 不支持在单个请求中动态切换数据库；
- 权威数据在 PostgreSQL，Mem0/pgvector 只是可重建的检索投影；
- 更完整的身份、数据库和接口设计见 [Identity 与 Agent Runtime 接入说明](./26-identity-runtime-integration.md)；
- 完整机器可读接口契约见 [OpenAPI](./api/openapi.yaml)。
