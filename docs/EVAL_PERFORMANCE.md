# BM25 长历史性能发现

实验均在本次新建、已清理的临时 PostgreSQL 18.4 上完成，没有连接已有服务。问题发生于第一次 LongMemEval-S 子集运行的第二题 `c6853660`，对应 801 个分块。第一次运行在客户端等待 120 秒后中断，PG 中该查询仍在执行；随后观察到约 193 秒的活跃查询和接近一个 CPU 核的占用。

这次运行未完成，不能用它的部分成绩作为完整子集成绩。`evals/reports/2026-11-06-local-v2/manifest.json` 标记为 failed；最终带硬上限的结果另存于 local-v3。

## 已证实的现象

- 源 SQL 来自 `MemoryRepository::search_bm25_projection`，未修改生产 SQL。
- 检查时 retrieval_documents 为 1683 行、retrieval_terms 为 115147 行；已有 autoanalyze 时间。因此不能把问题简单归因于“从未采集统计信息”。
- [generic plan](../evals/evidence/bm25-generic-plan.txt) 中 corpus_docs 估算 67 行；[实际执行](../evals/evidence/bm25-coffee-analyze.txt) 为 801 行。
- 同一 namespace 的单词 `coffee` 对照返回 29 个候选。EXPLAIN ANALYZE 显示 corpus 的 count 聚合执行 29 次，average/doc_lengths 的 HashAggregate 也执行 29 次；底层词项扫描读取 57893 行。总执行时间约 49.24 ms。
- [custom plan](../evals/evidence/bm25-custom-plan.txt) 也存在嵌套循环中的聚合，不能断言只强制 custom plan 就能解决问题。

这些证据支持“基数低估与统计聚合重复执行会放大多词/高频词查询成本”的判断。完整超时查询没有完成 EXPLAIN ANALYZE，尚不能将其全部耗时精确归因于某一个节点，也没有测量生产配置下的出现概率。

## 本次处理范围

Runner 给临时数据库角色设置默认 `statement_timeout=3000ms`，允许命令行在 1000～10000 ms 内显式调整；不修改查询算法、索引或生产配置。每种方案使用同一上限。

超时不会被重试或删题；该方法本题按空检索结果计入 Recall/MRR/nDCG，QA/拒答均记失败，并保存 `error=db_statement_timeout`。包含 BM25 的 Hybrid 整组记失败，没有偷偷降级成 Vector Only。

P50/P95 包含失败返回耗时，成功样本的 P95 另存于 `p95_success_ms`。触及上限时应理解为实际超时失败，不能据此声称完整查询只需要三秒。公开子集每题每组只测一次，分位数稳定性有限。

## 下一步修复建议

单独任务中先保存完整真实 bind 参数和 EXPLAIN，分别比较当前计划、显式物化 corpus/average/df、按 namespace 预计算统计。要求排名及 BM25 分值合同保持一致，再比较包含高频词、长历史和小 corpus 的延迟；有数据支持后才修改生产 SQL。当前评测交付保留问题，避免将性能修复与评测基建混在一起。

2026-11-07 第二阶段已经按上述方向实施受控对照，并完成统计物化与文档长度索引查询修复；原语句缓存保留。本文件保留第一阶段发现原文，后续证据与局限见 [EVAL_PHASE2.md](EVAL_PHASE2.md)。
