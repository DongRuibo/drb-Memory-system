# 长期记忆质量增强：待复核数据与离线实现

本轮按确认范围实施。收费 API 调用0，不读取模型 Key、不创建收费 Claim、不连接生产数据库、不提交/推送/合并。只修改评测代码，生产检索算法、治理规则、迁移、Reader/Judge Prompt、RRF权重与锁定LongMemEval14题均保持不变。

## 当前交付状态

- 新增48道查询：8类各6道，42道有答案、6道无答案。补充45条记忆，和原195条组合为240条；原数据文件不变。
- 新增24个治理场景：8类各3例。每例含明确操作输入、能力标签、数据库最终状态预期和理由。
- 全部标注为 `human_review=pending`。本报告不宣布用户已复核，也不将执行器夹具通过当成新数据集成绩。
- 完成Token A/B、阶段空上下文归因、治理专项、分类检索报告的Runner及JSON/CSV导出。
- 最终离线回归 **143项通过、0失败、0跳过**，其中新增24项（16项离线单测、8项隔离PG测试）。
- 本轮没有执行完整48题四组/24场景的正式新集评测；相关最终成绩保持空缺，等待用户复核。

人工复核入口：[48题表格](../evals/reports/memory-quality-v1-review-final/retrieval-review.md)、[24场景表格](../evals/reports/memory-quality-v1-review-final/governance-review.md)。同目录提供CSV；原始JSON在 `evals/datasets/context_quality_v1.json`、`governance_quality_v1.json`。这两份是自建工程数据，不是公开Benchmark。

## 只读审计结论与已有证据

审计基线为 `codex/memory-evaluation-system` / `9a5a438452502f71a207118ab104da5da5aa2071`。根目录与父目录没有实体AGENTS.md，本轮遵循对话提供的规则。历史架构图基于旧revision，只辅助理解，不作为当前部署证据。

| 已有结果 | 核实值 | 不能混用的口径 |
| --- | --- | --- |
| 工程留出集 | 195文档，20正例+4无答案 | 不是20题总量，也不是官方Benchmark |
| Qwen3-Embedding-0.6B | Recall@5=1.0000，MRR@10=0.9250，nDCG@10=0.9478 | 固定revision/1024维/L2；评测内精确cosine，不是生产ANN |
| BM25+真实Qwen3混合 | Recall@5=0.9500，MRR@10=0.8738 | 单路20、混合10+10；不可宣传混合一定优于单路 |
| 801块SQL性能对照 | P95 3047.53→199.82ms，超时3/6→0/6 | auto计划，两类查询各3次；失败计分，非生产SLO |
| 后续热查询 | 801块P95约83.33ms，n=40 | 另一轮环境/冷热合同，不能替代同组before/after |
| 空包修复 | TF-IDF可回答题字面空包9/10→0/10 | 仍有2/10无复核事实，全部事实进入Context仅6/10 |
| 治理 | 原11个探针符合预期 | 含一个专门复现语义改写去重缺口的探针，不能说11项能力都成功 |
| 历史PG集成测试 | 16通过、0失败、0忽略 | 有独立隔离库日志，不能将默认ignore算作通过 |

原始语义排名、801块性能记录均已只读重算。当前Repository源码Hash与第二阶段最终报告记录一致。语义延迟表排除了查询Embedding，编码P50约99.11ms另计。四组均对4道无答案题返回候选，不代表拒答或低幻觉。

现部署入口 `mem0_primary` 不经过这里的PG Candidate/Version/Head及Context Pack；本轮只验证 governed Repository ports。投影通过既有评测桥确认本地完成，不冒充Mem0/Elasticsearch远程同步、生产HTTP、认证入口、ANN或物理擦除验收。

## 数据集与待复核点

| 查询类别 | 数量 | 评测边界 |
| --- | ---: | --- |
| 实体和专有名词 | 6 | 相似人名、项目名、业务/监控端口 |
| 日期和时间 | 6 | 明确年月日、UTC/本地时间、周期及有效期 |
| 中文自然语言改写 | 6 | 不依赖原句复述的偏好/约束查询 |
| 多条件查询 | 6 | 同一记录的多个必要事实及跨记录关联 |
| 排除条件 | 6 | 明确排除同名人、测试环境、日志数、写入接口 |
| 新旧事实冲突 | 6 | 当前与明确历史时点分别标注，未决冲突保留双方证据 |
| 无答案查询 | 6 | 语料未提供目标属性；不参与正例召回/有效空上下文分母 |
| 多轮上下文查询 | 6 | 固定显式历史拼接，不宣称原生指代消解或LLM查询改写 |

治理类别为重复识别、冲突识别、明确纠正、版本替代、来源追溯、逻辑遗忘、Scope隔离、旧版本注入。优先复核：

1. g03/g06的同义等价预期：应去重/不冲突，但当前系统可能进入contested；保留为真实能力缺口，不能把复现缺口改为能力通过。
2. g07–g09提交的是明确Correction命令，不验证LLM从一句自然语言里识别纠错意图。
3. g14更正来源可直接指向user_correction SourceEvent，不要求伪造一次提取Candidate。
4. g16/g17/g24遗忘清空当前Head指针，历史Version仍有正文；不要求物理删除。
5. q31–q36是raw-history检索对照，旧文档作为独立记录存在；真正Version替换在治理集验证。
6. q43–q48的历史拼接属于已声明Adapter策略，不能写成系统原生多轮理解能力。

先前草稿的遗忘 `current_text` 字段错误地指向历史内容，现已改为null并单列 retained_history_text；项目输入补齐原规则要求的semantic_fact类型。修订理由在 `memory-quality-v1-review/draft-corrections.json`，能力标签未放宽。审核幂等键按既有SQL约束使用64字符哈希。初始失败日志保留，没有修改生产代码规避约束。

## Token A/B合同

A由真实 `authorized_hits` 得到当前身份可用的全部有效记忆，不用qrels筛选，不经过检索Top-K。B走真实搜索、RRF、canonical授权、现有透明相关性重排、compile及consume。两者用相同 `<memory version="…">正文</memory>` 格式，包含版本标识和分隔符。

采用DeepSeek-V3.2本地tokenizer，revision `a7e62ac04ecb2c0a54d736dc46601c5606cf10a6`，文件SHA256 `cd050be35cae877f8f0aa847f45aa87e23835a56ca32b29b28545597852784e5`，tokenizers==0.22.1，add_special_tokens=false。

这是渲染后的记忆payload精确Token数，不是提供方usage，也不包含系统提示、问题或聊天模板。生产编译仍按现有保守字符估算；本轮固定2048预算单位、最多10条，不擅自把生产计数器改成模型Tokenizer。历史4096预算另有原报告，不能混成同一实验。

逐题保存A/B文本、记忆数量、Token、证据保留、节省比例 `(A-B)/A`。汇总均值、P50/P95、总体比例 `1-ΣB/ΣA` 与逐题比例均值；A=0时比例null。失败题Token不伪造为0，单列失败数。节省率必须与必要证据保留率共同解释；大量干扰记忆会影响比例，不能外推生产收益。全量输入仅计数，不宣称一定适合某个Reader窗口。

## 空上下文与检索合同

保存source_union→RRF→preauthorization→rerank→output_limit→postauthorization→context_relevance→context_pack的ID、正文及分数快照。非hybrid_rerank组进入Context前仍复用已有QA的透明相关性重排；纯检索指标使用原始组内排名，因此Context表现不是“只改变召回通道”的严格消融。

正例分母42，负例分母6；零分母null。记录字面空包、无有效证据、部分证据及全部证据送达。实际收到Pack的字面空包率单列测量分母；请求失败另报，并计入整体有效上下文失败。阶段丢失率使用上一阶段已命中证据的条件分母，同时保存首次丢失位置。

来源/版本/Scope正确排除在治理组是成功，不算误排。授权阶段丢失能确定在哪一步，不一定能区分该SQL中的每个过滤谓词；不把它自动断言成某个权限规则缺陷。Context末段的预算遗漏保留原omission_summary，其余类型/阈值/limit组合损失保留阶段定位，不凭空补精确原因。

四组为BM25、TF-IDF、BM25+TF-IDF Hybrid、Hybrid+TransparentReranker；预算复用20总名额、混合10+10，输出10，权重仍1.15/1.0，RRF k=60。不用TF-IDF代表真实语义模型。新集真实Semantic尚未运行，旧Qwen3报告单列；本轮不启动外部模型服务。

复用已有规范Recall@1/@5、MRR@10、分级nDCG@10和分位数实现，不重新实现评分算法。按查询类别分组，负例Recall/MRR/nDCG为null，单独报告返回候选比例，不将返回候选等同于生成幻觉或拒答失败。模型Reader不在本轮执行范围。

## 治理统计合同

直接执行数据中的SourceEvent/Candidate/Correction/Review/Forget操作，断言真实Version/Head、来源关联、授权结果及实际Context。旧候选在投影删除之前测试，预编译Pack在状态改变后再次消费，不能只检查DB一列。

重复/冲突分别报告标签accuracy与TP/TN/FP/FN，另报包含最终DB状态的整例通过率。更新正确率覆盖明确纠正及审核版本替代；遗忘和Scope分别按3个场景统计。来源追溯、旧版本注入也分别报告，不合并成一个虚高总分。Scope每例都有owner可见的正对照，避免因所有人都读不到而“通过”。

## 验证证据

最终日志：`evals/reports/memory-quality-v1-review-final/tests-full-v3.log`，143项通过、0跳过，其中24项新增。真实PG测试创建独立Docker容器、tmpfs数据库、NOSUPERUSER/NOBYPASSRLS角色，并在退出时清理；没有读生产连接字符串。

新测试验证了默认rank结果不变、Trace阶段边界、全量授权不被Top10截断、来源链、审核拒绝、选择性遗忘、旧候选/旧Pack拒绝、用户/项目/租户Scope正反对照，以及“已知缺口仍记录为能力失败”。原11探针Runner直接复用回归，没有重新实现。

另有两个记忆的完整检索→Context→精确Token夹具，四组均使用真实PG和Tokenizer。逐题JSON在 `integration-fixtures-v3/retrieval-fixture-*.json`；治理夹具状态在同目录g*.json。夹具输出不是48/24全量新集成绩；测试“通过”不等于g03/g06能力通过。两者输出仍为passed=false，单测验证的是没有篡改标签。

最初两次PG测试分别因适配输入约束/草稿字段不准确而失败，日志分别为 `memory-quality-v1-review/tests-pg-initial.log` 和 `memory-quality-v1-review-final/tests-pg.log`。此前141项和142项运行也保留；最终版本另补了端到端Token夹具及真实空包/请求失败分母区分。

## 复现

本轮使用已经存在的本地Tokenizer与 `/tmp/drb-phase3-deps`，脚本不会加载.env、读Key或下载模型。

```bash
# 生成复核表，不需要数据库；输出目录须全新。
scripts/eval-memory-quality.sh prepare /tmp/memory-quality-review-new

# 完整离线回归；需要本机Docker和回环HTTP夹具权限。
scripts/eval-memory-quality.sh test

# 可选保存集成夹具JSON。
PYTHONPATH=/tmp/drb-phase3-deps MEMORY_QUALITY_PG_TEST=1 \
  MEMORY_QUALITY_TEST_ARTIFACTS=/tmp/memory-quality-test-traces-new \
  python3 -B -m unittest evals.tests.test_quality -v

# 待标注复核后执行完整新集；未确认时报告仍标pending。
scripts/eval-memory-quality.sh run evals/runs/memory-quality-v1-001
```

缺Tokenizer或固定依赖会明确失败，不降级字符数。缺Docker不能计作PG通过。Runner只接受自身创建的drb_eval回环数据库；没有生产数据库参数或收费模型入口。现有预算账本与Judge V4封存资产均不重置。

完整新集运行后输出retrieval/governance逐题JSON/CSV、authorized-full-memory、失败记录、manifest、summary，以及context-token、empty-context、retrieval、governance四份独立JSON/Markdown汇总。未完成运行标failed，不生成完成成绩；缺失/重复查询结果不能汇总成成功。

## 文件范围与简历边界

新增两份数据集、独立协议、quality.py、test_quality.py、eval-memory-quality.sh、本说明与复核/测试证据；已有文件仅增量修改 `evals/harness/src/main.rs` 和 `evals/README.md`。原32个文件、历史报告、旧Claim/账本和锁定集的Hash核验见复核目录protection-check.json。

目前可推荐的真实指标仍来自既有工程实验：195文档/20正例上的Recall@5=100%、MRR@10=0.9250；801块同夹具P95约3.05s→0.20s、超时3/6→0/6；历史16项隔离PG集成测试通过。新增Token节省率、有效空上下文率、重复/冲突/更新/遗忘/Scope准确率须等复核及全量运行后再补，不提前填写或作为生产成绩。

最终计分回归还验证：重复识别标签与来源完整性分别计分；来源边缺失会使整例失败，但不反向抹去正确的重复识别标签。来源专项则要求全部应保留来源都链接到真实Version，不能只验证已有边的引用合法性。
