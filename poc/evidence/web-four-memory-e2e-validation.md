# Web 四类记忆统一演示验收记录（2026-07-29）

## 结论

本机真实链路已跑通“自然对话自动记忆 → 新对话按需使用 → 项目事实自动更新并提示 →
指定经历 Forget”。本记录不包含 API Key、数据库密码、模型请求原始 ID
或回答正文。

## 运行配置

- Memory Service：`127.0.0.1:8080`
- Demo Web API：`127.0.0.1:8090`
- Web：`localhost:3000`
- 权威存储：PostgreSQL `memory_core`
- 可重建投影：Mem0 collection `memories_qwen3_1024`，generation
  `qwen3-semantic-20260729b`
- Embedding：`Qwen/Qwen3-Embedding-0.6B`，1024 维，L2
- Rerank：`Qwen/Qwen3-Reranker-0.6B`
- 对话与自动提取：Kimi Code `k3`
- Demo User Scope：`semantic-demo`
- Demo Project Scope：`lazytt-memory-system-demo`

## 验收步骤与证据

### 1. 一条自然消息自动形成四类记忆

Turn `10706ef3-84a2-4aa9-b883-6ce6bb26273b` 完成，生成 5 条正式记录并在同一轮
Context Pack 中使用：

| 类型 | 语义键 | Scope | 结果 |
|---|---|---|---|
| `preference` | `user.response_style.technical.conclusion_first` | user | accepted |
| `profile_fact` | `user.role` | user | accepted |
| `profile_fact` | `user.tooling` | user | accepted |
| `project_fact` | `project.architecture.authority_store` | project | accepted |
| `episodic_summary` | `user.experience.outbox.duplicate_consumption` | user | accepted |

Turn 持有 Context Pack `68bdd9f3-cdb3-537c-acdc-979b81a130e7` 和 Outcome Receipt
`4ef96f83-2f9c-5b83-b842-02fd2c0eb2a8`，模型为 `k3`。

### 2. 新对话按显式意图召回

对独立会话的问题执行真实 Semantic Search + Rerank，5 条记忆全部返回。低于通用语义
阈值但被用户明确请求的 `profile_fact`、`project_fact` 由确定性查询意图策略保留，原始
Rerank 分数未被放大或改写。

### 3. 项目事实自动追加版本并提示

Turn `ef31dc00-c33a-4cea-be33-583af25f5ff5` 将“改用 CockroachDB、不再使用
PostgreSQL”归一为新项目事实。底层 Policy 先建立冲突 Candidate，再由 Web Runtime
以稳定幂等键自动接受；浏览器没有接受/拒绝按钮，只显示“已自动更新记忆”的结果提示。

自动更新 Operation `5cf49d97-6a4c-5140-8ebe-c2b659fbd18e` 完成：

- Record 保持 `b2b148a9-4c1a-55a1-8694-8157048d7856`；
- Head 为 `active`，revision 为 2；
- v1 保留 PostgreSQL；
- v2 `0a40d392-4f5b-500f-982a-d34591e1692d` 当前值为 CockroachDB；
- 当前 Turn 的 Context Pack `69f9cb75-b9f4-5513-b224-9af2340ce686` 精确绑定 v2，
  Outcome Receipt 为 `c500c753-91d4-5a5d-8072-b477955ec103`；
- 回答真实使用 CockroachDB v2 与 Preference v1，共 2 条记忆。

### 4. 指定经历 Forget

Turn `235a3eae-fb46-4b47-bc00-951bb43465c2` 对 Outbox 经历执行结构化 Forget，
Operation `59f8ab19-86b3-5fd8-8755-1842d888d461` 使记录进入 `forgotten`、revision 1。
该 Turn 的 Context Pack 不包含被忘记的经历，其他 Preference、Profile 与 Project
记录保持 active；Turn 与 Outcome Receipt 均完成。

## 回归结果

- Rust workspace 非 ignored 测试共 76 个通过、0 失败；其中两个 Kimi Client 测试使用
  本机 loopback mock 验证请求契约和流式增量顺序。
- PostgreSQL 项目 Scope/冲突集成测试通过；Demo Chat Store 的 ignored 持久化测试另行
  对真实本机 PostgreSQL 执行并通过，同时验证 Record ID 与精确 Version ID 成对落库。
- Web `npm test`、`npm run lint` 和生产构建通过；两个渲染测试覆盖交互式 Agent 页面和
  “应用框架固定、仅对话区滚动”的长对话布局。
- Memory OpenAPI、Demo Web OpenAPI 与事件 AsyncAPI 三份 YAML 契约均可解析。
- 浏览器目视验收无 console warning/error：应用与 `body` 高度均等于 viewport；页面
  显示 Project v2 和自动更新提示，确认按钮数与冲突卡片数均为 0。
- 第一轮回答的来源抽屉仍显示当时使用的 PostgreSQL v1，不会被后来更新的
  CockroachDB v2 覆盖，证明 Turn 证据绑定的是精确 Memory Version，而不是当前 Head。

## 已知边界

- 这是可信的本地产品演示，不等于生产发布批准。
- 外部身份与项目 membership authority、OIDC/JWKS、HA、灾备、容量压测、正式
  tokenizer、告警和合规验收仍属于 Production Gate。
- 当前 Web Demo 对通过类型、权限、来源和置信度检查的低风险冲突采取自动 supersede；
  生产环境应按领域风险配置哪些 semantic key 允许自动更新。
- 引导页的 Forget 按钮把自然语言展示与明确的目标 Record ID 绑定，避免让模型猜测
  删除目标；自由输入中的任意自然语言删除意图尚未作为通用控制面能力实现。
