# 下一轮真实模型实验执行条件与预算草案

这是第二阶段的历史草案。第三阶段已将提供方、固定协议和报价落实到 [离线准备报告](EVAL_PHASE3_PREPARATION.md)：14题、最多130次收费请求，最坏11.467264元、拟硬上限12元，尚未获消费授权。下文假设单价的10.153元/144次不作为当前执行预算。

本轮实际模型 API 调用与费用均为 0。下述是待实施、待明确预算授权的计划，不是已获得的模型结果。

## 已有接口能做什么

| 组件 | 真实代码接口 | 当前状态 |
|---|---|---|
| BM25 | MemoryRepository.search_bm25_projection | 已运行，统计物化及按版本索引读取长度已验证 |
| TF-IDF | evals/memory_eval/baselines.py | 词袋/中英词项 cosine；不能称为语义 Embedding |
| Hybrid | weighted_reciprocal_rank_fusion | 当前是 BM25 1.15 + TF-IDF 1.0、k=60 |
| 透明重排 | TransparentReranker | 词法相关性与检索分数的加权规则，不是语义模型 |
| 真实 Embedding | tools/retrieval_models_server.py `/v1/embeddings`；Mem0 的向量索引适配 | 有代码，未在本轮启动或验证模型 |
| 真实 Reranker | HttpReranker、KimiReranker；本地 `/v1/rerank` | 有代码，未在本轮运行真实模型 |
| 真实 Formation | Kimi/OpenAI-compatible MemoryExtractor | 有提取实现；本轮没有模型质量证据 |
| 真实回答生成 | Memory Service 外部 Agent / Reader | 本轮 Reader 是单句抽取规则；仍需独立、受预算控制的真实 LLM Adapter |

本地网关已有 Qwen3-Embedding-0.6B 与 Qwen3-Reranker-0.6B 的实现，分别支持 last-token pooling/L2 和 yes/no logits；建议先复用这一对，不新增多个模型。默认配置记录的 Embedding revision 为 `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`、1024 维；Reranker 为 `e61197e`，正式运行应解析为完整提交哈希，并保存实际加载清单。

Embedding 请求须区别 query/document 的 instruction；网关支持 `model/input/dimensions/input_type`，返回 embedding 数组，健康清单记录真实模型和 revision。`EmbeddingProbe` 主要用于 readiness，不是通用检索 API；governed 生产索引仍经 Mem0Adapter，不可把探针响应当成生产向量索引已经验证。

HttpReranker 将 `[memory_type=...]` 和正文交给模型，要求每个候选恰好一个 index/score、有限的 `[0,1]` 分值及正确 model 标识；默认专用重排最小分数为 0.10，但 Context Pack 还有 0.40 的闸门。两者不是可随意交换的概率尺度。下一轮必须记录两级筛选和遗漏原因，在开发集确定分数合同，不能看到公开答案后调整阈值。

网关目前会按 max_length 截断输入。真实实验必须记录实际 tokenizer、每项 token 数和截断标志；不能默默截断证据后只汇报 QA 分数。0.6B 模型以双字节权重估算约 1.2GB/个，这只是权重算术估算，不是实测总内存；框架、激活、批处理、MPS/CUDA 缓冲另计，需做本地硬件探测。

最小接入可以放在 evals：通过现有本地 HTTP 接口生成冻结向量，评测 Adapter 做精确 cosine，继续使用现有 canonical 授权、RRF 和 Context Pack。这样可先比较真实模型本身，无需修改生产检索架构。此时必须标注为评测内 exact search，不能声称验证了 Mem0/ES 的 ANN、持久化或故障恢复；若要覆盖生产路径，再单列隔离投影实验。

## 第一轮应固定的实验合同

1. 本轮已检查的 12 题作为诊断/开发子集，不再称为未见测试。再从官方 S 文件预先抽取一组与这 12 题不同的题目作为锁定测试，保留题型分层、ID、源 SHA、会话重合统计。原生 500 题与各类子集分开报告。
2. 保持同一历史、分块、Scope、相关性标注、召回名额 20 和输出 10；Embedding 文档向量一次计算并缓存，cache key 必须绑定正文哈希、模型完整 revision、instruction、维度和归一化。
3. 先做纯检索四组：BM25、真实 Embedding、二者 Hybrid、Hybrid+真实 Reranker。纯检索指标不依赖回答器，不将 TF-IDF 与真实模型混为一谈；TF-IDF/透明重排可保留为额外控制组。
4. 检索与 QA 分轨：纯排序消融复用同一 Hybrid 候选；QA 则先明确 Context Pack 的分数合同。原始 cosine 不能越过相关性转换直接进入 0.4 闸门。若使用 rank-only Context 作为诊断，必须单列，不冒充生产默认策略。生产策略下则完整保留模型阈值、预算遗漏及候选阶段差异。
5. Reader 只收到问题、必要时间信息及最终授权上下文；回答须带版本/来源引用并支持拒答。金答案、has_answer、review snippets 仅供评分/Oracle。Oracle Evidence 和实际检索各跑一组，用于拆开 Reader 与检索问题。
6. 保持严格 EM 作为可复现诊断；正式 LongMemEval QA 另用官方固定 judge prompt/model。judge 输出与人工抽查分别记录，不能用当前严格 EM 冒充官方 Accuracy，也不能用调宽字符串匹配替代官方标准。
7. 官方 judge 脚本存在 backoff 行为，不能原样无限重试。实现预算适配层：调用前预留最坏 token 成本；限制每次输入/输出、调用总数、总金额、超时与重试数，记录失败和 usage，密钥只在进程环境中读取，不写报告。超过输入上限时标记预算失败，不删题或截短参考答案来迁就上限。
8. 真正的自然语言 Formation 单独评测。先对冻结的 24 条 Formation 样例比较现有规则与真实提取器；公开 S 原文分块 RAG 的结果不能被写成“全历史已由 LLM 提取并治理”的成绩。要测试完整 Formation，需要另行测量全部历史 turn 数和调用/token 预算。

## 小规模预算估算

以下使用**规划单价上界**：输入 10 元/百万 token、输出 40 元/百万 token，纯用于预算计算，**不是供应商报价**。正式运行前必须核对实际报价；若实际单价超过该上界，则重新计算，不执行。默认重试 0，失败调用仍占预留额度。

| 项目 | 次数硬上限 | 每次输入/输出 token 上限 | 规划金额上限 |
|---|---:|---|---:|
| 本地 Embedding + Reranker | 一次冻结语料建索引；每题一次重排 | 以实测 tokenizer/显存确定批量，记录全部截断 | 外部模型 API 0 元；不含本机电力或租用 GPU |
| 12 题 × 4 检索方案 + 12 次 Oracle Reader | 60 | 8192 / 512 | 6.144 元 |
| 对应官方 judge | 60 | 4096 / 32 | 2.5344 元 |
| 24 条独立 Formation 样例 | 24 | 4096 / 512 | 1.47456 元 |
| 合计 | 144 | 同上 | 10.153 元；建议另设总硬上限 20 元 |

本地模型也设置计算上限草案：最多 12000 个文档向量、12 个查询向量；embedding batch≤32、HTTP 请求≤400、总运行≤60 分钟；rerank≤12 次、每次≤20 个候选（≤240 对）、总运行≤10 分钟。失败重试为 0，超限保留失败题，不延长为无限任务。实际冻结子集若超过该语料上限，应先重新核算，不静默删历史。这些限制尚待 Adapter 实现。

这是下一轮小样本试验的成本上界草案，不授权本轮发生费用，也不包含完整 500 题或全历史 Formation。若使用收费 Embedding/Reranker，还需先对冻结语料按真实 tokenizer 计数、确认按 token/次/文档对计费的规则，单列预算，不能套用本地 0 元。

## 开跑前仍缺的具体条件

- 一个能实际加载并返回完整 revision 的语义模型运行环境；校验依赖锁、设备、维度、instruction、截断与资源峰值。
- 明确测的是 governed 还是默认部署的 mem0_primary；前者的 Candidate/Version/Head 和后者的 Mem0 原生形成流程不能混写。本轮只验证 governed Repository 端口，下一轮还需新建隔离的服务/向量投影，不复用已有部署数据。
- 一个真实、受调用数/token/金额硬限制的 Reader/judge Adapter；当前实现没有这层执行能力，不能只靠环境变量声称预算受控。
- 经开发集核对的分数合同和固定 Context 策略，避免 raw cosine、RRF、词法相关性、cross-encoder 分数混用。
- 新锁定的公开题目、二次人工复核的事实/证据标注，以及 rubric、短实体、算术和时间问题的分开统计。
- 评测内 exact cosine Adapter 与缓存模型不匹配检查；若宣称生产路径效果，还需隔离向量投影生成、持久性/重建等测试。现有 PG 与词法测试通过不等于 Mem0/ES 语义路径通过。
- 保存完整阶段 trace、SQL/模型各段耗时、失败、不被删除的样本分母，并明确区分 raw-history RAG 与真实 Formation+治理。

这些条件完成后，可以开展一轮可信的小规模真实模型公开数据对照；仍不能仅凭 12～24 题作出生产容量或排行榜结论。
