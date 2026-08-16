# Documentation Map

本目录是 LazyTT Memory System 的规范性文档入口。

## 文档权威顺序

发生冲突时按以下顺序处理：

1. 已接受的 [ADR](./adr/README.md)；
2. [OpenAPI](./api/openapi.yaml) 与 [领域事件契约](./events/asyncapi.yaml)；
3. 本目录中的编号规范；
4. 根目录 [CONTEXT.md](../CONTEXT.md) 中的领域用词；
5. `doc/` 中的历史调研材料。

任何实现如果偏离 1～3，必须先更新 ADR 或规范，不能仅靠代码注释改变系统语义。

## 阅读顺序

1. [目标、边界和成熟度](./00-overview.md)
2. [领域模型与不变量](./01-domain-model-and-invariants.md)
3. [系统架构与信任边界](./02-architecture-and-trust-boundaries.md)
4. [Memory Policy](./03-memory-policy.md)
5. [写入、读取、修正和遗忘流程](./04-write-read-correct-forget-flows.md)
6. [数据模型与事务约束](./05-data-model-and-invariants.md)
7. [Provider 与投影契约](./06-provider-and-projection-contract.md)
8. [Memory Context Pack 契约](./07-context-pack-contract.md)
9. [安全、隐私与威胁模型](./08-security-privacy-and-threat-model.md)
10. [一致性、故障和恢复](./09-consistency-failure-and-recovery.md)
11. [评测规范](./10-evaluation-specification.md)
12. [测试策略](./11-test-strategy.md)
13. [可观测性、SLO 与容量](./12-observability-slo-and-capacity.md)
14. [部署与运行手册](./13-deployment-and-operations-runbook.md)
15. [开发/发布准入清单](./14-release-readiness-checklist.md)
16. [Agent Runtime 集成契约](./15-agent-runtime-integration.md)
17. [真实语义模型与自动记忆](./16-semantic-models-and-automatic-memory.md)
18. [交互式 Memory Agent V2](./17-interactive-memory-agent.md)
19. [项目与代码架构](./18-project-and-code-architecture.md)
20. [记忆分类与跨项目数据合并方案](./19-memory-taxonomy-and-data-merge-plan.md)
21. [Recall Explain 合并结果与实体图决策](./20-recall-explain-and-entity-graph-decision.md)
22. [后端与 Demo 拆仓及本地联调边界](./21-repository-split-and-local-integration.md)
23. [后端 Docker 本地部署](./22-backend-docker-deployment.md)
24. [记忆系统会议介绍](./23-memory-system-meeting-overview.md)
25. [如何理解当前系统架构](./24-how-to-understand-current-architecture.md)

演示应用自身的 HTTP 契约见 [Demo Web OpenAPI](./api/demo-web-openapi.yaml)。

## 规范用词

- **MUST / 必须**：不满足即不符合系统契约；
- **SHOULD / 应该**：通常必须满足，偏离需要记录理由；
- **MAY / 可以**：可选实现；
- **POC gate**：完成后才允许确认技术选型；
- **MVP gate**：完成后才允许集成到 Agent Runtime；
- **Production gate**：完成后才允许处理真实用户长期记忆。
