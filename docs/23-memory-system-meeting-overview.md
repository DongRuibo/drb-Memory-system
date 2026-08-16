# LazyTT 记忆系统会议介绍

状态：会议简版，基于 2026-07-31 当前源码、测试记录和本机验收结果整理。

> 本文用于开会快速说明项目进展。文中的百分比是按“能力与实现责任”做的阶段性估算，
> 不是代码行数统计，也不代表已经通过生产上线评审。

## 1. 记忆系统

### 一句话介绍

LazyTT 记忆系统是给 Agent 使用的长期记忆后端。

它可以从自然对话中识别值得长期使用的信息，经过权限、类型、来源和版本治理后保存；
当 Agent 再次回答问题时，只取出“与当前问题相关、当前仍然有效、当前用户有权读取”的
记忆交给模型。

它不是简单地把所有聊天记录存进向量数据库。

```text
自然对话
  → 提取记忆候选
  → 权限和安全检查
  → PostgreSQL 保存正式版本
  → Mem0 / BM25 / 时间通道建立检索投影
  → 按当前问题召回相关记忆
  → 形成可审计的 Context Pack
  → Agent 使用这些记忆回答
```

### 为什么需要这个系统

普通大模型调用结束后，通常不会自动记住用户之前说过的内容。直接保存全部聊天又会带来
信息污染、权限混乱、版本冲突和删除困难。

LazyTT 解决的是以下问题：

- 什么信息值得长期记住；
- 这条记忆属于用户、项目、租户还是 Agent；
- 同一个事实发生变化时，哪个版本是当前版本；
- 哪些历史版本需要保留用于审计；
- 当前问题应该使用哪些记忆；
- 用户要求忘记或清除后，如何确保旧内容不再进入回答。

## 2. 开发人员

### 当前研发方式

- 当前 GitHub 主要提交账号：`ruibo-dong_lazyx`；
- 研发方向：Agent 系统、Memory Service、检索与上下文工程；
- 开发方式：核心开发人员主导架构和功能实现，AI 编程工具辅助编码、测试和文档整理；
- 验收原则：不以模型口头回答作为完成证据，以代码、数据库状态、测试和真实运行结果为准。

### 前后端分工

| 部分 | 仓库 | 职责 |
| --- | --- | --- |
| 记忆后端 | `lazyaix/lazytt-memory-system` | 正式记忆、版本、权限、检索、Context Pack、Forget/Erase |
| 演示前端 | `lazyaix-dev/lazytt-frontend-console` | 聊天页面、用户切换、记忆档案、证据面板、会议演示 |

正式 Agent 后续只需要接入记忆后端；Web 前端是可选的测试和演示工具。

## 3. 当前综合完成度：约 70%

这里的 70% 表示“核心功能闭环已经形成，可以运行和演示”，不表示已经达到生产上线标准。

| 方向 | 当前状态 | 简单说明 |
| --- | --- | --- |
| 核心记忆闭环 | 已完成 | 能记住、检索、更新、忘记并审计 |
| 自动记忆 | 已完成 | 自然对话可自动形成四类正式记忆 |
| 时间和结构化内容 | 已完成 | 能记录事件时间、时区、精度和受控 JSON 结构 |
| 多用户与项目范围 | Demo 已完成 | Alice/Bob/Carol 可演示个人隔离和项目共享 |
| Context Pack | 已完成 | 每轮只向模型提供相关且获授权的记忆快照 |
| Docker 下载运行 | 已完成 | 后端和演示前端可分开下载、构建和启动 |
| 正式 Agent Runtime 集成 | 未完成 | 当前主要接入的是 Demo Agent |
| 短期会话上下文 | 未完成 | 历史聊天已保存，但还没有按最近若干轮传给模型 |
| 生产安全和运维 | 未完成 | OIDC、HA、灾备、监控、容量和合规验收仍需补齐 |

### 如何理解当前阶段

```text
架构设计        已完成
核心功能闭环    已完成
本地真实演示    已完成
他人下载测试    已完成
正式 Agent 接入 进行中
生产上线        未达到
```

## 4. 实际验收结果

### 4.1 已跑通的真实业务链

当前已经跑通：

```text
自然对话
  → 自动提取记忆
  → 类型和 Scope 判断
  → PostgreSQL 正式入库
  → Embedding / BM25 / 时间检索
  → Reranker 重排
  → Context Pack
  → Kimi 回答
  → Outcome Receipt
  → Forget 后不再注入
```

这条链路不是静态页面或模拟数据，实际经过 PostgreSQL、Mem0、Embedding、Reranker 和
真实对话模型。

### 4.2 四类记忆验收

一段自然介绍可以形成以下记忆：

| 类型 | 例子 | 主要用途 |
| --- | --- | --- |
| 用户偏好 `preference` | “技术问题先给结论，再列要点” | 调整回答方式 |
| 用户资料 `profile_fact` | “用户是 Agent 研发工程师” | 个性化背景 |
| 事实知识 `semantic_fact` | “当前项目使用 PostgreSQL” | 项目或领域事实 |
| 经历摘要 `episodic_summary` | “曾用 event_id 解决 Outbox 重复消费” | 相关历史经验 |

### 4.3 时间检索验收

系统能够区分：

- 事件什么时候发生；
- 消息什么时候发出；
- 记忆版本什么时候创建；
- 某个事实从什么时候开始生效。

已经验证“昨天、前天、7 月、7 月份、明确日期、季度”等时间查询。近期真实回归中，
询问“我 7 月份干过什么”能够召回 2026-07-15 的 Outbox 经历，并展示模型实际使用的
记忆版本和事件时间。

### 4.4 检索效果验收

固定合成评测集的阶段性结果：

- 15 条正查询：Recall@3 = `1.000`；
- 15 条正查询：MRR = `1.000`；
- 3 条负查询：错误命中数为 `0`；
- Forget 后：被忘记内容注入数为 `0`。

以上结果证明当前方案能够完成受控演示和回归测试，但数据集规模较小，不能直接代表
生产环境中的最终质量。

### 4.5 多用户和项目验收

- Alice 的个人记忆不会被 Bob 读取；
- Alice 和 Bob 属于同一项目时，可以读取项目共享事实；
- Carol 不属于该项目时，不能读取该项目事实；
- 浏览器不保存 Memory Service JWT 或模型 API Key；
- Forget 后由 PostgreSQL 权威状态立即阻断读取，Mem0 随后异步删除投影。

### 4.6 部署验收

目前支持以下本地运行方式：

| 模式 | 适用情况 |
| --- | --- |
| `cloud` | 使用硅基流动 Embedding/Reranker，下载测试最方便 |
| `cpu` | 模型放在 Docker CPU 容器内，完全本地但速度较慢 |
| `cuda` | Linux + NVIDIA GPU |
| `host-mps` | Apple Silicon Mac 使用宿主机 MPS 加速 |

后端和前端已经拆分，可以只下载后端作为正式 Agent 的记忆模块；需要体验功能时，再单独
下载 Web 演示前端。

## 5. 已拥有能力

### 5.1 记忆写入与治理

- 从自然对话自动提取记忆候选；
- 四种正式记忆类型；
- User、Project、Tenant、Agent 四种访问范围；
- Secret、PII 和 Prompt Injection 边界检查；
- 精确去重和多个来源合并；
- 事实变化形成新版本，不直接覆盖旧版本；
- 内容冲突进入 Review/Contested 保护状态；
- Outbox 保证权威数据库与检索投影最终一致。

### 5.2 记忆检索

- Qwen3 Embedding 语义检索；
- PostgreSQL 中文 BM25；
- 本地词法匹配；
- Event Time 时间范围检索；
- 多路结果加权 RRF 融合；
- 独立 Reranker 重排；
- Provider 或 Reranker 不可用时的可观测降级；
- 检索后回 PostgreSQL 再做权限和当前版本检查。

### 5.3 记忆生命周期

- Inspect：查看当前记忆和完整版本历史；
- Correction：追加新版本并移动当前 Head；
- Forget：立即停止读取，同时保留审计链；
- Erase：物理擦除可清除的正文；
- Expiry：到期后自动失效；
- Export：按用户导出可用记忆；
- Projection Rebuild：从 PostgreSQL 重建检索投影。

### 5.4 Agent 上下文

- 每个 Task/Attempt 生成独立 Context Pack；
- Context Pack 绑定服务器时间、时区、权限快照、记忆版本和用途；
- 模型只收到当前问题、运行时上下文和本轮获授权的长期记忆；
- 证据面板展示与模型 User Payload 一致的上下文快照；
- 每轮保存使用的 Memory Version、Context Pack 和 Outcome Receipt。

### 5.5 演示与使用

- 大模型式聊天页面；
- Alice/Bob/Carol 用户切换；
- 右侧记忆档案；
- 类型、范围、事件时间和结构化内容展示；
- 版本时间线；
- 当前回答使用的记忆证据；
- 事实更新、Forget 和物理清除入口；
- Docker 一键构建和健康检查。

## 6. 自研占比：约 65%

这里的 65% 是按核心能力的设计和实现责任估算。

主要自研内容包括：

- 记忆领域模型：Source、Candidate、Record、Version、Head、Ledger；
- 四类记忆和 typed Scope 规则；
- Memory Service HTTP API 和 `memory-client` SDK；
- PostgreSQL Schema、Migration、事务和 RLS；
- Policy、权限交集、幂等、去重、冲突和 Review；
- Correction、Forget、Erase、Expiry 和 Export；
- Transactional Outbox 与 Projection Worker；
- 多路检索编排、RRF 融合和两次 canonical 授权；
- 中文时间解析、Event Time 和结构化内容协议；
- Context Pack、Usage Receipt、Retrieval Receipt 和 Outcome Receipt；
- Web Demo 的记忆编排、证据面板和多用户演示逻辑；
- Docker Profile、启动脚本、测试脚本和验收材料。

系统的核心价值主要集中在“如何治理记忆、如何限制模型、如何确保可删除和可审计”，这些
部分由 LazyTT 自己实现。

## 7. 开源框架占比：约 25%

开源项目提供底层数据库、Web/Rust 框架和模型能力，但不决定什么是正式记忆。

| 开源组件 | 当前用途 |
| --- | --- |
| PostgreSQL 17 | 权威记忆、版本、Ledger、Context Pack、回执 |
| pgvector | Mem0 的向量存储基础 |
| Mem0 OSS | 可重建的向量检索投影 |
| Qwen3 Embedding | 语义向量生成 |
| Qwen3 Reranker | 候选相关性重排 |
| Rust | 后端主要开发语言 |
| Axum / Tokio | HTTP 服务和异步运行时 |
| SQLx | PostgreSQL 访问和 Migration |
| Next.js / React | Web 演示界面 |
| Docker Compose | 本地部署和组件编排 |

即使 Mem0 或模型被替换，PostgreSQL 中的正式记忆、版本和审计链仍然保留。

## 8. 第三方占比：约 10%

第三方服务主要提供模型推理能力：

| 第三方服务 | 当前用途 | 是否可替换 |
| --- | --- | ---: |
| Kimi Code `k3` | 自动提取候选、Demo Agent 回答 | 可以替换，但需重新验收协议和质量 |
| 硅基流动 | Cloud Profile 的 Embedding/Reranker API | 可以切换到本地 CPU/CUDA/MPS |

第三方模型不能直接修改正式记忆。它们只能提出候选或计算检索分数，最终是否接受、谁能
读取、当前使用哪个版本，仍由自研 Memory Service 和 PostgreSQL 决定。

## 9. 未实现功能

### 9.1 下一阶段优先补齐

1. **短期会话上下文**
   - 最近若干轮聊天窗口；
   - 会话滚动摘要；
   - 短期对话、会话摘要和长期记忆联合编译；
   - 模型实际收到的短期上下文同步进入证据面板。

2. **正式 LazyTT Agent Runtime 集成**
   - Runtime 身份和 Attempt 生命周期；
   - 正式 Prompt 注入；
   - MemoryPort 契约测试；
   - 回答 Outcome 与 Agent Task 关联。

3. **生产身份与权限系统**
   - OIDC/JWKS；
   - 外部 Membership Authority；
   - Key Rotation；
   - 主动撤权事件和传播时限验证。

### 9.2 生产上线前必须补齐

- TLS/API Gateway 和正式 Secret Manager；
- 固定发布镜像、SBOM、镜像签名和漏洞扫描；
- 大规模检索质量、延迟、并发、内存和成本评测；
- SLO、监控、告警、Dashboard 和值班流程；
- PostgreSQL Backup/PITR；
- RPO/RTO、灾难恢复和 Tombstone Replay 演练；
- Projection Generation 的生产级 canary/rollback 演练；
- HA、多副本、容量规划和故障注入；
- 隐私、安全和 Production Readiness Review。

### 9.3 暂时不在当前范围

- 把原始聊天全部当作长期记忆；
- 从其他项目导入历史用户记忆；
- 完整 GraphRAG 或知识图谱推理；
- 自动生成并发布 Tool/Skill；
- 用记忆替代真实业务数据库、权限系统或审批系统。

## 10. 剩余约 30% 的建议拆分

| 后续工作 | 建议占比 | 目标 |
| --- | ---: | --- |
| 短期会话上下文与正式 Agent 集成 | 10% | 从“记忆 Demo”走向完整 Agent 上下文 |
| 正式身份、安全和撤权 | 8% | 从固定 Demo 用户走向真实组织用户 |
| 监控、HA、备份和灾备 | 6% | 从本地可运行走向稳定服务 |
| 大规模评测和发布验收 | 6% | 用数据证明质量、性能和上线条件 |

## 11. 会议总结

可以用下面四句话结束介绍：

1. LazyTT 已经完成了一个能真实运行的受治理长期记忆闭环，不是静态原型；
2. 系统的核心自研价值是权威数据、版本、权限、检索治理、删除和审计；
3. PostgreSQL、Mem0、Qwen 和 Kimi 都是可替换组件，不能绕过 Memory Service 修改正式记忆；
4. 当前约完成 70%，下一步重点是短期会话上下文、正式 Agent 集成和生产化建设。

## 12. 详细材料入口

- [项目与代码架构](./18-project-and-code-architecture.md)
- [记忆分类与机制](./19-memory-taxonomy-and-data-merge-plan.md)
- [后端与 Demo 拆仓边界](./21-repository-split-and-local-integration.md)
- [后端 Docker 部署](./22-backend-docker-deployment.md)
- [发布准入清单](./14-release-readiness-checklist.md)
- [Web 演示讲解流程](./demo-web-presentation-plan.md)
- [Web 四类记忆验收](../poc/evidence/web-four-memory-e2e-validation.md)
