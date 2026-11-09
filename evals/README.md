# Memory 评测

面向 `governed` 库的离线基建。现有默认部署 `mem0_primary` 不走本轮治理链；本轮不会改变部署模式。

下文前两阶段的默认Runner继续保持零模型API；第三阶段使用单独的、默认关闭收费执行的入口，见本文末节。

## 复现

需要 Python 3.12+、Rust/Cargo 1.97+、Docker，以及本机的 `postgres:18.4-bookworm` 镜像。Python 只用标准库。第一次可下载依赖，之后固定 lock 离线构建：

```bash
docker pull postgres:18.4-bookworm
CARGO_TARGET_DIR=/tmp/drb-memory-eval-target cargo fetch --locked --manifest-path evals/harness/Cargo.toml
python3 -m unittest discover -s evals/tests -v
python3 -m evals.memory_eval --output evals/runs/local-001 --seed 20261106 --budget 20 --repeats 3
```

必须从仓库根目录执行，output 必须不存在。不能传入生产数据库 URL、已有服务 URL 或 API Key。Runner 只创建带 `drb-eval-*` 名称和标签的临时 PG，回环随机端口、tmpfs、非超级用户/非 BYPASSRLS 角色，结束后仅删除自己创建的容器。模型 API 调用和费用硬上限均为 0；不会加载任何 `.env`。

`DATABASE_URL` 即使存在也不会传给桥。桥只接受 Runner 使用的 EVAL_DATABASE_URL，固定角色、回环地址和 `drb_eval_*` 库名；没有外部模型调用功能。下载公开数据属于独立命令，不是付费模型调用。

公开实验使用经典 LongMemEval-S cleaned 的固定 12 题子集，包含六种题型和拒答组。下载约 277 MB；源文件不进 Git：

```bash
mkdir -p evals/downloads
curl -L --fail --max-time 600 --max-filesize 330000000 \
  https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/main/longmemeval_s_cleaned.json \
  -o evals/downloads/longmemeval_s_cleaned.json
shasum -a 256 evals/downloads/longmemeval_s_cleaned.json
# 预期：d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442
python3 -m evals.memory_eval \
  --output evals/runs/longmemeval-s-001 \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json \
  --variant s --limit 12 --seed 20261106 --budget 20 --repeats 3 \
  --statement-timeout-ms 3000
```

最终真实运行使用 `/tmp/drb-eval-public/longmemeval_s_cleaned.json`，其内容哈希与上述一致。更换源文件或 seed 必须产生新报告。Adapter 也支持官方 Oracle/M 文件，但需匹配 `--variant`；Oracle 只有证据会话，成绩不可当作 S/M。

## 目录与实验合同

| 路径 | 用途 |
|---|---|
| `memory_eval/runner.py` | Runner、临时 PG 生命周期、CLI 执行、逐题结果、汇总与 Markdown 报告 |
| `memory_eval/metrics.py` | Recall、MRR、规范 nDCG、P50/P95、一对一 Formation P/R/F1、QA 精确匹配/拒答/旧事实 |
| `memory_eval/baselines.py` | 无网络的 TF-IDF、规则提取、抽取式 Reader |
| `memory_eval/longmemeval.py` | 官方 JSON 校验、固定题型抽样、隔离标签的 Adapter |
| `harness/` | 独立 Cargo 工作区，调用现有 Repository/RRF/TransparentReranker/Context Pack |
| `datasets/memory_v1.json` | 40 文档、36 检索查询、24 Formation、16 QA 的自建回归集 |
| `tests/` | 评分、标签隔离、边界分块、超时计分、报告防覆盖测试 |
| `reports/2026-11-06-local-v3/` | 最终真实结果；旧 v1/v2 为失败运行，不能汇总进最终成绩 |
| `reports/2026-11-07-local-repeat/` | 另一全新数据库的本地复跑；全部排名与质量指标一致 |
| `reports/verification.json` | 结果数量、复跑一致性、已有文件保护及测试计数 |
| `evidence/` | 长历史 BM25 查询计划和性能证据 |

四组使用同一 corpus/query/qrels；默认总召回名额 20：BM25 20、Vector 20、Hybrid 为 BM25 10+Vector 10，融合最多 20，输出最多 10。Hybrid+Rerank 采用相同 Hybrid 召回配置和预算，增加现有透明重排器。参数预算可用 10～50 的偶数，不能逐题调参。

每组独立执行召回，因此公开长历史发生超时时，实际候选可用性会不同；不能将公开子集组间的超时差异解释为单独增加 Rerank 的因果收益。无超时的本地复跑已验证全部排名和质量可复现；Head 闸门消融则确实使用同一份旧候选。

Vector 是 TF-IDF cosine（log TF、corpus-only smoothed IDF、L2，英文词/CJK 双字词），不是真实语义 embedding。Hybrid 的 RRF 权重 1.15/1.0、k=60；Rerank 是生产 TransparentReranker，不是 Qwen/Kimi 神经网络重排。没有擅自改生产接口来加入选择检索通道的开关。

自建检索每组一次预热，随后固定种子随机打乱方法和题序测 3 轮；36 题仍是 36 个独立样本。公开子集为每题每组单次测量。延迟含 Python/Rust IPC、查询向量化、PG 搜索、融合/授权/重排，排除灌库、Context Pack、Reader；不同方案的索引构建耗时没有纳入每查询延迟。

Formation 仅是评测用规则基线，严格比较人工事实别名，不代表现有 LLM 提取器能力。自建 QA 先对原始自然语言做规则提取，再另库隔离 namespace 写入；公开 QA 使用全部角色的原文分块，明确绕过生产 LLM Formation。Reader 仅能看最终 Context Pack，不能读原始全历史或答案标签。

默认 SQL 语句上限 3000 ms，超时保留题目、计为失败；Hybrid 不因 BM25 超时偷偷降级。报告的延迟包含失败返回，成功 P95 单独保存在 JSON。详情见 [性能证据](../docs/EVAL_PERFORMANCE.md)。

## 输出与解释

每次保留 `manifest.json`（状态、seed、代码/数据哈希、模型标识、预算、镜像）、`retrieval.json`、`formation.json`、`qa.json`、`governance.json`、`ingestion.json`、`failures.json`、`summary.json`、`report.md`、`bridge.log`。公开运行另存 `longmemeval-selection.json`。异常运行 status=failed，保留已完成部分，不能当完整分数。

拒答题无正例相关文档，Recall/MRR/nDCG 为 null，不放入正例分母；拒答正确率单独报告。没有旧事实题时 stale_fact_error_rate 为 null；动态更新/遗忘题另成组，避免虚报大样本 0% 错引。

治理的 11 个探针包含一个已知缺口：语义改写被置为 contested。该探针断言通过表示缺口被准确复现，不能声称“11 项能力全部通过”。Head 闸门消融使用同一旧候选集，对比直接信任候选与 canonical 过滤；没有修改生产开关。

语义向量/神经 Rerank、生产 LLM Formation/生成回答、官方 judge、Mem0/ES 传输和生产并发均未验证。不能把自建集/12 题子集写成官方完整 Benchmark、排行榜成绩或生产 SLO。

进一步阅读：[代码审计](../docs/EVAL_AUDIT.md)、[公开数据与许可](../docs/EVAL_BENCHMARKS.md)、[面试材料](../docs/EVAL_INTERVIEW.md)、[第三方声明](THIRD_PARTY_NOTICES.md)。

## 第二阶段：诊断与最小修复

第二阶段继续使用同一独立分支和零模型 API 预算。生产变更仅在 Repository：三个 BM25 统计 CTE 物化、按 Version 利用现有索引计算文档长度，保留预备语句缓存；已有冲突审核时，新冲突明确返回 409 并回滚，不扩展多候选审核。

新增 `phase2.py` 保存阶段全文、Oracle Evidence、参数化执行计划、SQL 次数与数值参考对照。原始数据和答案不变；QA 在原始相似度后先走现有相关性重排，避免把 cosine 直接当成 Context 的相关性分数。Reader 及严格答案评分没有更改。

从根目录运行，所有 output 必须是新目录：

```bash
python3 -m unittest discover -s evals/tests -v
python3 -m evals.memory_eval.phase2 performance \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/p2-perf
python3 -m evals.memory_eval.phase2 residual \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/p2-cache
python3 -m evals.memory_eval.phase2 cardinality \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/p2-unseen-tenant
python3 -m evals.memory_eval.phase2 diagnosis \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/p2-qa
python3 -m evals.memory_eval.phase2 holdout --output evals/runs/p2-holdout
python3 -m evals.memory_eval.phase2 pg-tests --output evals/runs/p2-pg
python3 -m evals.memory_eval --output evals/runs/p2-regression --seed 20261106 --repeats 3
python3 -m evals.memory_eval.phase2_report
```

`performance` 冻结原 SQL、仅物化 SQL、最终 SQL，包含多租户统计布局、100/400/801 块、自动/generic 计划和独立 Python BM25 参考；`residual` 检查缓存，`cardinality` 使用合成多租户夹具重现新租户基数低估，不作为公开分数。公开题目选择固定使用第一阶段的 seed=20261106；第二阶段 CLI 的 seed=20261107 控制方法顺序和 bootstrap，不改变该 12 题。新运行另存 `longmemeval-selection.json`；本轮已完成实验的源 SHA 与输入一致性记录在 `reports/phase2-validation.json`。

使用 pg_stat_statements 的是新建临时 PG；不会为已有实例装扩展或修改配置。普通 SQL 上限 3000 ms，cardinality 的 EXPLAIN 单列 10000 ms，不混进普通延迟。仅在明确指定的临时库运行旧 ignore 测试，不删除原保护。报告生成器默认读取下表已有证据；自定义运行可用 `--performance`、`--diagnosis`、`--holdout`、`--pg-tests` 和 `--output` 指定目录。中间缓存/物化试验仍引用已保存目录，不需要付费或重跑模型。

实际证据目录：

| 目录 | 内容 |
|---|---|
| `reports/phase2-performance-final/` | 原/物化/最终 SQL 对照、计划、资源与无词项边界 |
| `reports/phase2-residual/` | cached/uncached 及 generic/custom 对照 |
| `reports/phase2-cardinality-lengths/` | 未见租户压力夹具、约 4716 万次 join 排除及最终索引计划 |
| `reports/phase2-longmemeval/` | 仅物化的中间完整重放，仍有超时 |
| `reports/phase2-longmemeval-final-v2/` | 禁缓存中间试验，仍有超时，非最终生产策略 |
| `reports/phase2-longmemeval-final-v3/` | 最终代码的完整 12 题重放和阶段 trace |
| `reports/phase2-holdout-final/` | 修复前冻结的 195 文档/24 查询、原/新 SQL 等价性、bootstrap 区间 |
| `reports/phase2-pg-tests/` | 首次 15/16 的真实失败日志 |
| `reports/phase2-pg-tests-lengths-final/` | 最终 16/16 集成测试日志 |
| `reports/phase2-regression-final/` | 原 36 查询、四组、三轮回归 |
| `reports/phase2-unit-tests-final.log` | 64 个 Rust 单元测试，另行运行的 PG 测试不算作跳过通过 |

先前的启动/校验失败和中断运行均保留并标识状态，不混入成绩。数学参考对近似浮点并列采用 1e-12 容差，实际排序与检索/答案指标不变。留出集由同一开发者编写，未调参，但不是独立团队盲测；旧集的标注问题另存 `datasets/memory_v1_errata.json`。

完整结果与逐题分类见 [第二阶段报告](../docs/EVAL_PHASE2.md)，当前仍缺条件和预算草案见 [真实模型计划](../docs/EVAL_REAL_MODELS_PLAN.md)。这些修改仍仅适用于 governed 路径，不表示当前 mem0_primary 部署已验收。

## 第三阶段：离线准备与待授权的真实模型实验

协议、分母、价格、局限及审批边界见 [第三阶段准备报告](../docs/EVAL_PHASE3_PREPARATION.md)。新公开锁定集是14题，开发/测试的题目与全部会话重叠为0，源数据仍使用上文固定SHA的经典LongMemEval-S。该14题不是官方完整Benchmark或排行榜。

仅准备和测试，不会调用收费API：

```bash
python3 -m venv /tmp/drb-eval-phase3
/tmp/drb-eval-phase3/bin/python -m pip install --no-cache-dir -r evals/requirements-phase3.txt
/tmp/drb-eval-phase3/bin/python -m evals.memory_eval.phase3 assets --output /tmp/drb-phase3-assets-001
/tmp/drb-eval-phase3/bin/python -m unittest discover -s evals/tests -v
/tmp/drb-eval-phase3/bin/python -m evals.memory_eval.phase3 prepare \
  --assets /tmp/drb-phase3-assets-001 \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json \
  --output evals/runs/phase3-preparation-001
/tmp/drb-eval-phase3/bin/python -m evals.memory_eval.rescore3 \
  --assets /tmp/drb-phase3-assets-001 \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json \
  --output evals/runs/phase3-dev-rescore-001
python3 -m evals.memory_eval.performance3 \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/phase3-perf-001
python3 -m evals.memory_eval.performance3 --startup-only \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json --output evals/runs/phase3-startup-001
```

`assets`只下载固定revision的公开tokenizer（不下载权重）；hash或大小不符即失败。`prepare`保存计划seal、逐题数据量、精确Embedding文本Token、最大调用/费用、模型/协议/源码hash。旧集复核不会预测新测试集。若没有tokenizer资产，对应单元测试明确跳过，不能计通过。

免费本地模型运行环境单列在 `requirements-phase3-models.lock`，为本次macOS arm64/Python3.12实测环境；其他硬件应另建配置和计划。不会修改系统Python或默认生产模型服务：

```bash
/tmp/drb-eval-phase3/bin/python -m pip install --no-cache-dir -r evals/requirements-phase3-models.lock
HF_HOME=/tmp/drb-phase3-hf HF_HUB_DISABLE_XET=1 HF_HUB_DISABLE_IMPLICIT_TOKEN=1 \
  /tmp/drb-eval-phase3/bin/python tools/retrieval_models_server.py \
  --host 127.0.0.1 --port 19882 --device mps --max-length 4096 \
  --embedding-model Qwen/Qwen3-Embedding-0.6B \
  --embedding-revision 97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3 \
  --embedding-dimensions 1024 --disable-reranker
# 另一个终端，仅跑已用过的自建工程集，不读新公开锁定集：
/tmp/drb-eval-phase3/bin/python -m evals.memory_eval.semantic3 \
  --assets /tmp/drb-phase3-assets-001 --url http://127.0.0.1:19882 \
  --output evals/runs/phase3-semantic-holdout-001
```

模型权重下载和本地计算不产生外部模型API费用，但有网络、磁盘和硬件时间成本。health必须匹配模型revision、维度、L2、设备、dtype、max_length、instruction和依赖；不就绪不降级伪装Semantic。输入超过4096 Token时停止，网关usage=0的占位字段不当成真实Token消耗。缓存保存在输出目录，默认不纳入Git。

收费入口是 `python3 -m evals.memory_eval.phase3 models-run`。缺少 `--plan` 和 `--approval` 会在数据库、网络或读取密钥前拒绝。计划最长130次，费用上界11.467264元，拟硬上限12元；这不是已授权消费。**本轮没有生成有效批准文件。**

只有用户明确授权并确认16个校准标签后，才创建绑定最终plan seal的批准记录（approved、calibration_labels_human_reviewed、reviewer、plan_seal），再提供专用环境变量 `MEMORY_EVAL_API_KEY`。不在命令行或文件中写Key。执行前再核对官方价格与账户可用性；计划或代码变化会使旧批准无效。

```bash
# 以下仅在获得明确授权后运行；现在不要创建有效approval文件。
/tmp/drb-eval-phase3/bin/python -m evals.memory_eval.phase3 models-run \
  --assets /tmp/drb-phase3-assets \
  --longmemeval evals/downloads/longmemeval_s_cleaned.json \
  --plan evals/reports/phase3-calibration-v3/plan.json \
  --approval <用户明确授权后生成的批准记录.json> \
  --semantic-url http://127.0.0.1:19882 --output evals/runs/phase3-models-001
```

先完成免费就绪检查，再消耗一次性批准；2次smoke、16次Judge校准（至少15/16整例标签完整匹配且关键错误为0）通过后才跑新14题。每次最坏预留先落盘，失败可能计费则保留预留并停止，重试0，不能覆盖账本重新消费。保存traces中的实际输入/截断/引用、逐题结果、原始模型输出、usage、费用、失败、版本与Markdown/JSON报告。Reader不接触参考答案；Judge和Oracle单独接触必要标注。

当前实际证据：`reports/phase3-dev-rescore/`、`phase3-performance-v2/`、`phase3-startup-attribution/`、`phase3-compatibility/`。`phase3-performance/`为随机端口变化导致的首次夹具失败，保留并不计入结果。当前可审议计划使用下方 `phase3-calibration-v3/`；先前 `phase3-preparation/` 和 `phase3-handoff/` 保留为历史，运行时hash检查会拒绝将其当当前方案。

## 收费前最后核验

当前审议材料为 [v3修正与最终门禁](../docs/EVAL_JUDGE_CALIBRATION_V3.md)，[16例清单](../docs/JUDGE_CALIBRATION_REVIEW.md)保留v2历史内容；当前计划使用 `reports/phase3-calibration-v3/plan.json`，旧handoff和phase3-protocol-review-v2保留为历史。没有改校准标签或锁定14题，也没有生成人工通过/消费批准。

预算只允许POSIX主线程顺序执行；同一工作区、同一锁定集+Reader的持久实验claim禁止并发、换输出路径和重启后清零续跑，已有账本不覆盖。角色次数56/56/16/2、总130、硬金额12元；费用与报价同源。每次先落盘预留，失败未知计费保留，重试0，总网络时限45秒。实际HTTP慢滴流回归需要本机回环端口权限；不需要模型Key或生产数据库。

```bash
PYTHONPATH=/tmp/drb-phase3-deps python3 -m unittest discover -s evals/tests -v
PYTHONPATH=/tmp/drb-phase3-deps python3 -m evals.memory_eval.phase3 prepare \
  --assets /tmp/drb-phase3-assets \
  --longmemeval /tmp/drb-eval-public/longmemeval_s_cleaned.json \
  --output evals/runs/phase3-calibration-v2-001
```

人工标签改变需明确修订并重封计划；不得使用锁定14题的成绩反调Prompt、RRF或评分规则。完整测试和保护校验在 `reports/phase3-calibration-v3/`。

## 校准集v2最终整理

已按用户复核意见创建独立 `datasets/judge_calibration_v2.json`，v1原文和锁定14题未变；16例原有标签不变，仅3例事实/措辞消歧义。最终人工确认仍待完成，收费调用0。规则、逐字段差异及实验限制见 [v2规则](../docs/EVAL_JUDGE_CALIBRATION_V2.md) 和 [16例最终清单](../docs/JUDGE_CALIBRATION_REVIEW.md)。

每例accurate/supported全部匹配才算1例通过，拒答必须显式supported=null；不按32个标签平均。16个有效唯一结果、至少15例完整匹配、关键错误0才放行；具体关键检查随v2数据封存。旧计划因校准文件与Judge Prompt变化失效，预算标识和12元上限不重置。

当前离线prepare使用 `--assets /tmp/drb-phase3-assets`；原示例的`-001`目录是历史环境，如不存在不要误认为模型缺失。不得执行models-run，须先获得最终确认和明确消费授权。

## 校准集v3最后修正

当前使用 `datasets/judge_calibration_v3.json`，只补充正例室内偏好事实和用户指定的18条关键门禁，参考答案与标签不改；v1/v2文件及锁定14题均保留。协议钉住v3 SHA，预算标识不变。全部相关离线测试、20组模拟关键错误及新计划见 `reports/phase3-calibration-v3/`。等待最终授权，不调用收费API。


## 首次获授权的真实执行：校准门禁失败后停止

当前真实记录见 [完整报告](reports/phase3-real-models-20261107/report.md)。2次可用性检查及16次真实Judge校准已完成；15/16整例匹配，但partial_answer的supported误判属于关键错误，所以没有执行正式LongMemEval14题、Reader或正式Judge。usage公示价估算0.035568元，实际调用18次，未重试。

原模型、Prompt、V3 SHA、RRF和锁定集均不改；已存在的paid-budget claim与原账本保留。不要删除claim、更换输出目录或修改评分来续跑。可离线读取报告中的calibration-v3.json和judge-calibration.json，使用calibration_summary重算门禁；所有18次输入SHA均已与实际账本核对，Trace包含完整输入、原始输出、usage与费用。

正式质量指标为null。历史工程集或开发子集结果不替代未执行的公开锁定集；真实Reader效果、Oracle、拒答和幻觉均未验证。下一轮实验安排须另行确认。


## Judge Evaluation Contract V4（免费离线，待确认）

首次真实校准的完整性/支持性混淆已形成独立V4 Prompt及适配器，未改V3或Reader/检索。见 [根因、最小差异及预算](../docs/EVAL_JUDGE_CONTRACT_V4.md) 和 [14例新对照复核表](../docs/JUDGE_V4_CONTROLS_REVIEW.md)。原16例标签及15/16、关键错误0门禁不变；旧真实结果回放仍失败，V4真实效果保持null。

```bash
python3 -m evals.memory_eval.judge4 prepare --output evals/runs/judge-v4-offline-001
```

该入口只准备输入和费用提案，没有models-run，不读Key或发模型请求。当前封存为reports/judge-contract-v4-locked；同轮judge-contract-v4及judge-contract-v4-final为较早的离线草案。收费集成与新授权尚待完成；不得复用旧授权/清零旧账本。提案新增14次独立对照，总144次；含首次0.035568元的累计最坏10.966768元。建议先确认32次校准及对照阶段（最坏1.240064元），不得默认为已授权正式112次。

## 当前：V4 第一阶段执行器已集成，等待 32 次明确授权

以上 V3/旧 V4 入口与计划作为历史保留。当前使用独立的 `judge4_stage1` 和 `configs/judge_v4_stage1.json`，详见 [执行器、门禁与预算报告](../docs/EVAL_JUDGE_V4_STAGE1.md) 及 [封存证据](reports/judge-v4-stage1-sealed/report.md)。本轮收费调用 0，不读取 Key，不创建真实收费 Claim。共享客户端校验顺序修正后，旧 `judge4 prepare` 的源码 hash 会拒绝漂移；不修改旧封存配置来继续执行。

```bash
# 免费封存；目录须全新，不读取凭据或调用模型。
python3 -m evals.memory_eval.judge4_stage1 prepare --output /tmp/judge-v4-stage1-review-new
PYTHONPATH=/tmp/drb-phase3-deps python3 -m unittest discover -s evals/tests -v
python3 -m compileall -q evals/memory_eval evals/tests
git diff --check
```

仅准备 2 次 smoke、16 次原校准、14 次新对照，总 32 次，费用上界 ¥1.240064；含历史费用累计 ¥1.275632。原 16 例要求至少15/16完整匹配、关键错误0，新14例要求14/14、关键错误0，分别报告。关键错误或门禁已不可能通过时立即停止；两道通过也只产生证据交接，不会自动调用正式112次。

未来获得绑定当前 plan seal 的明确授权并复核价格后，才可创建有效批准文件和提供 `MEMORY_EVAL_API_KEY`：

```bash
# 本轮禁止执行；--approval 不能复用旧授权。
python3 -m evals.memory_eval.judge4_stage1 run \
  --plan evals/reports/judge-v4-stage1-sealed/plan.json \
  --approval <明确授权后创建的第一阶段批准记录.json> \
  --output evals/runs/judge-v4-stage1-real-001
```

真实执行固定 Claim 目录及阶段键，不允许更换输出目录/复制批准/重启重开预算；旧账本不一致时停止核账。每次先预留、持久化输入/原始输出、校验后结算，网络/schema/usage失败保留最坏预留，45秒总时限、重试0。完整模拟32次、两道失败停止、并发与崩溃回归均有封存日志；模拟标签不代表真实V4质量。联合Judge仍看到参考答案和证据，`support_only_view`未实现调用级隔离。
