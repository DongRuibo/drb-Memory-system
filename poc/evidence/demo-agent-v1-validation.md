# Demo Agent v1 本地编排验证

验证日期：2026-07-28

本文件记录客户端临时选择逻辑阶段的历史验收。服务端 Context Pack 改造后的最新
本地证据见 [Demo Context Pack 本地端到端验证](./demo-context-pack-validation.md)。

## 验证范围

本次验证使用真实的 PostgreSQL、Memory Service、Outbox Worker 和固定 Mem0
runtime；Mem0 使用本地无语义 Embedder，Agent LLM 边界使用仓库提供的确定性
OpenAI-compatible Chat Completions 假服务。

本次验证没有使用真实供应商凭据，也没有调用外部 Kimi 服务。因此它证明的是
Agent/Memory/Chat Completions 的 HTTP 编排与权限生命周期，不证明 Kimi 模型质量、
外部认证、配额或真实网络行为。

## 运行配置

```text
Memory Service: http://127.0.0.1:8080
PostgreSQL:     127.0.0.1:55432
Mem0:          http://[::1]:8888
POC Embedder:  http://host.docker.internal:18080/v1
POC Chat:      http://127.0.0.1:18081/coding/v1
Model field:   k3
```

Chat 假服务只记录合成 request ID、模型名、reasoning effort、memory context
条数和 user payload SHA-256；不记录问题、记忆正文或 Authorization。

## 验证结果

一次完整运行成功通过：

1. 明确记忆请求提交后，canonical 状态为 `memory_accepted`，Head revision 为 `0`；
2. Outbox Worker 将 Projection 推进到 `ready`，attempts 为 `1`；
3. 全新 Task/Attempt 搜索到目标 canonical Memory，`memory_context_count=1`；
4. Agent 的第一份回答按照偏好展示【原理】和【示例】；
5. Forget 提交后 canonical 状态立即为 `forgotten`，Head revision 变为 `1`；
6. 在等待 Provider 删除之前，全新 Task/Attempt 已无法读取目标 Memory，
   `memory_context_count=0`；
7. Agent 的第二份回答没有获得该偏好上下文；
8. 后台 Projection 最终进入 `deleted`，attempts 为 `1`。

本次合成运行的目标 `memory_record_id`：

```text
6603a1bb-1d6d-5d4d-8436-796e158e1e90
```

## 同轮工程验证

```text
cargo check --workspace                       PASS
cargo clippy --workspace --all-targets -- -D warnings
                                                  PASS
cargo test --workspace                        PASS
python3 -m py_compile POC servers             PASS
bash -n demo scripts                          PASS
git diff --check                              PASS
repository long-format API-key scan           NO MATCH
```

Workspace 测试共覆盖：

- Demo Agent 编排与两个独立 Task/Attempt；
- Kimi OpenAI-compatible endpoint、Authorization、model、effort、
  `max_completion_tokens` 和 `prompt_cache_key`；
- Kimi Base URL 固定与 loopback 测试例外；
- Memory JSON 编码和 `untrusted_memory_data` 边界；
- Mem0 Provider error body 不进入错误日志；
- 现有 Domain、Repository、Adapter 和 Memory Service 回归测试。

## 当时尚未完成

- 使用真实语义 Embedder 和固定评测集验证多记忆相关性。

真实 Kimi 验收与后续 Context Pack 再次验收见
[Demo Agent v1 真实 Kimi 验收](./demo-agent-v1-kimi-validation.md)。
