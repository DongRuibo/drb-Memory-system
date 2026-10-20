# 2026-10-20 DeepSeek 真实恢复验收

基线 `2b48ba9`，隔离分支 `codex/resume-pgvector`。本轮没有下载模型或拉取镜像；
本机原工作区与原 Docker 部署保留。所有新增聊天、抽取、Episode、检索和性能输入均为合成数据。
本轮根据用户授权调用 DeepSeek，不能再称为“纯离线验收”。

## 实现与实际通过项

1. 保留原 Kimi 路径，增加 DeepSeek 聊天方言：官方 HTTPS 端点、配置模型、max_tokens、
   关闭 thinking、SSE 流式用量；流式和非流式协议回归均通过。参数依据
   [DeepSeek 官方接口文档](https://api-docs.deepseek.com/api/create-chat-completion/)，并做了真实请求。
2. 修复 global 偏好被旧技术回答归一化改写的问题，增加独立 global conclusion-first 语义身份。
   现有事实分类和 contextual 默认规则保留，未批量改写旧数据。
3. 新增默认关闭的答案适用性复核，本轮启动器显式启用。Qwen 做本地检索和候选排序，
   DeepSeek 复核已授权候选是否适用；非 global 最终适用性分数来自复核，Qwen 用于同分排序。
   复核只允许完整的既有 ID 集合与合法分数；请求失败、响应截断或非法 ID 时拒绝记忆检索。
4. 策略身份同时绑定 primary 版本和复核模型，以固定长度哈希满足 128 字符约束。
   配置变更不复用旧 Context Pack 排序身份；没有修改旧 migration checksum。
5. 提供隔离启动器、22 例抽取驱动、聊天治理驱动、Episode 驱动、16 例适用性驱动和延迟测量。
   原 `.env` 只定向读取 DeepSeek 密钥／模型，不写回、不整份复制；前端不接收后端密钥。

完整复现入口见 [演示说明](../../docs/resume-demo.md)。

| 检查 | 实际结果 |
| --- | --- |
| 工作区普通 Rust 测试 | 150 passed，0 failed；21 项数据库测试默认 ignored |
| PostgreSQL 集成 | 17 Repository＋3 Memory API＋1 Demo Chat，通过 |
| Support 最终针对性回归 | 2 项通过，覆盖拒绝非法结果／连接失败及版本标识长度 |
| Python 启动／脱敏替身 | 4 项通过 |
| Clippy | 全工作区所有 target 通过；仅豁免 Elasticsearch 基线已有 manual_clamp 告警 |
| 真实聊天治理 | 10 项断言通过，6 次真实回答；包含跨会话回忆、修正、新版本、争议拒读、审核、个人记忆隔离、遗忘后的实际 Prompt 零记忆 |
| 抽取与安全边界 | 22/22：通用／场景偏好、资料／项目事实、日期、相对时间、无事件时间、混合分类、计划／问题／不确定／注入／合成凭据等 |
| Episode | 10 个真实已完成轮次；单个 threshold job 覆盖 1～10，一次尝试完成，4 条 Proposal；助手无来源猜测未成为记忆 |
| 补充适用性回归 | 16/16，通过实际 Memory API、Qwen、DeepSeek 与权限／时间过滤链 |
| 浏览器视觉／交互 | **未完成**：本机来源访问被浏览器安全授权检查拒绝；未换浏览器或截图工具绕过 |

HTTP 聊天结果确认模型实际收到 `prompt_context.memory_context` 和 current version，
不以 HTTP 200 或模型口头“记住了”代替记忆成功。Forget 验证模型实际 Prompt 数组为空。
SSE 由真实 Demo 后端消费并持久化回答；没有将其描述成已完成浏览器流式交互验收。

## 固定检索集

数据集仍为 v2.2.0，20 条文档、19 个正例、5 个负例；正文、标签、范围及阈值均未修改。
排序策略新增 DeepSeek 适用性复核，因此这是新策略成绩，不是本地 Qwen 单独达到。

| 指标 | Qwen＋新分类（前阶段） | Qwen＋DeepSeek 复核（本轮） |
| --- | --- | --- |
| Recall@3 | 1 | 1 |
| MRR | 1 | 1 |
| 负例查询命中率 | 0.2（1/5） | 0（0/5） |
| 遗忘后注入 | 0 | 0 |
| 正例返回结果无关注入率 | 0 | **0.2083** |
| 数据集定义的 Release Gate | FAIL | PASS |

剩余取舍：既定门槛不约束正例结果无关注入率。新策略保留了更多可能适用的技术回答偏好，
也返回了少量非目标事实。4 个查询出现额外文档：q02、q05、q06、q07；详见原始结果。
其中偏好是否适用与固定集仅标注目标答案的口径并不完全一致，但也不能据此忽略额外事实。
本轮没有重标数据、加查询黑名单或放宽阈值；这项精准性问题仍需独立样本验证和进一步优化。

16 例补充集在前阶段发现失败后构造，是诊断回归，不是独立留出基准。
旧 11/16 是模型逐对分数测试，本轮 16/16 是完整链路；方法不同，不能按同条件模型提升宣传。

## 延迟与运行资源

设备：Apple M5 Pro，24 GiB RAM，macOS 27.0.1；检索模型在 Docker CPU 运行。
本轮实际加载固定 revision 的 Qwen3 Embedding-0.6B（1024 维、L2）与 Reranker-0.6B。
模型容器约使用 4.85 GiB，Docker 可用约 7.75 GiB；不是主机原生 MPS 性能。

- 模型容器重启到 health：17,035.27 ms，来自已有只读缓存重新加载权重；不是首次安装、
  下载或清空操作系统磁盘缓存后的冷启动。
- 3 次预热后 30 次 Memory search：P50 **1,846.36 ms**，P95 **2,242.19 ms**，全部成功且未降级。
- 条件：3 条合成项目事实、同一查询重复、并发 1、loopback HTTP、包含本地检索与云复核。
  不包含聊天生成和记忆抽取，也不是容量、并发、长上下文或生产 SLA 测量。

DeepSeek 为供应商模型别名 `deepseek-flash`，本轮没有可固定的云端权重 revision。
没有用 Token 数估算未核实的账单；实际费用以供应商账单为准。

## 初次失败与修复证据

- 初次聊天跨会话找不到姓名，同时旧归一化把 global 表达缩成技术回答偏好；保留 chat-live.json。
- 初次复核策略版本串超过回执／Context Pack 的 128 字符限制，导致无记忆降级；保留
  chat-live-v2.json，修为固定长度身份后 chat-live-v3.json 的 10 项断言全部通过。
- 抽取 v1 初报 21/22，因为合成凭据被安全边界 HTTP 400 拒绝，而驱动原先只接受零候选。
  v1.1 明确该例预期为安全拒绝，重新跑 22/22；这不是模型效果提升或绕过安全拒绝。
- Episode 首次驱动错误地期待“无新轮次时再请求整理”返回旧任务；实际契约为 HTTP 409。
  清理还发现 contested 记录必须先审核再 Forget。修正驱动后对同一真实 10 轮运行重新核对，
  单任务覆盖、已完成状态、无助手猜测和无重复范围均通过；未把测试预期错误改成产品成功。

## 证据索引

所有文件位于 [本轮证据目录](deepseek-recovery-2026-10-20/validation-summary.json)。

- [真实聊天与 10 项断言](deepseek-recovery-2026-10-20/chat-live-v3.json)
- [22 例抽取](deepseek-recovery-2026-10-20/live-extraction-v11.json)、[数据集](deepseek-recovery-2026-10-20/extraction-dataset-v11.json)
- [10 轮 Episode](deepseek-recovery-2026-10-20/episode-live-v11.json)
- [16 例适用性](deepseek-recovery-2026-10-20/applicability-live-v1.json)
- [固定集比较报告](deepseek-recovery-2026-10-20/comparison-report.md)、[指标](deepseek-recovery-2026-10-20/metric-results.json)、[原始返回](deepseek-recovery-2026-10-20/raw-results.json)、[运行身份](deepseek-recovery-2026-10-20/evaluation-manifest.json)
- [30 次原始延迟](deepseek-recovery-2026-10-20/latency-live-v1.json)、[模型重启](deepseek-recovery-2026-10-20/model-restart-latency.json)

更早的离线、数据库、投影契约、重启持久化和 Web 构建证据仍保留；本轮没有再次声称
Web 浏览器视觉验收、生产容量、高可用或物理全副本擦除已经完成。

测试清理：三个本轮合成租户共 88 个 Head 已 forgotten，旧租户未完成的投影删除已通过
临时隔离 Worker 补齐；待处理 Outbox 为 0。Episode、账本和测试回执作为审计历史保留，
没有执行物理擦除或删除数据卷。演示服务保留供后续人工或获授权的界面验收。
