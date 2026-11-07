# 第三阶段：真实模型评测的离线准备

收费前最后核验及预算修复已另存 [最终协议检查](EVAL_PROTOCOL_FINAL_CHECK.md)，人工16例见 [复核清单](JUDGE_CALIBRATION_REVIEW.md)。当前执行计划使用phase3-calibration-v2；最新规则见 [v2校准说明](EVAL_JUDGE_CALIBRATION_V2.md)，本文保留前一轮准备证据。

本阶段尚未获得收费调用授权。Reader/Judge 的真实成绩未产生，不能把准备完成写成真实 LLM 评测完成。免费本地 Embedding 的就绪/工程集验证单列，公开锁定测试集尚未预测。第一、二阶段原始结果和原有32个文件保留。

## 已锁定的测试集

`evals/datasets/longmemeval_phase3_lock.json` 锁定经典 LongMemEval-S cleaned 的14题：六种题型各2题，另2题拒答，正例分母12。题目ID、抽样种子20261108、源SHA、预处理及seal均在文件中。

选择方法是按题型对 `SHA256(seed, question_id)` 排序，排除开发集题目及与开发集任意历史会话重叠的题。未按答案、预测成功率或历史长度筛选。与旧12题的题目、答案会话、全部历史会话重叠均为0。初稿发现8个共享历史会话，尚未预测就收紧了隔离条件；初稿另存 evidence/phase3，不作为测试协议。

最终seal：`c609995fec7af4e3615cdf8e1aa333b1651aad43d355dbe7fe49720b2bc0e876`。全量输入为11315块。保留所有角色、干扰历史、日期；按时间输入，空白折叠、每1000字符无重叠分块。官方无时区日期按UTC解释。不改金答案，不根据has_answer筛普通检索语料。

这是分层公开子集，**不是官方完整500题、V2或排行榜成绩**。2题/题型和2道拒答不足以支持统计显著性或泛化结论。原12题仅作开发诊断；自建195文档/24查询仅作工程对照。

## 指标和统计分母

| 指标 | 计算/分母 | 边界 |
|---|---|---|
| Session Recall@1/5/10 | 每题前10个返回块映射会话、去重后取前K；命中相关会话数/标注相关会话总数，再对可回答题求均值 | 不是召回K个完整会话；同一候选预算下实际会话数可能不足K |
| Evidence Recall@1/5/10 | 前K块中官方has_answer标记块数/该题全部has_answer块数，按有证据标注的正例宏平均 | 官方标记turn拆块的代理；**不是人工复核的事实级召回**。缺标注返回null，并报告覆盖率 |
| Context Evidence Coverage | 实际送入Reader、与原证据仍相同的前缀字符数/该题全部标记证据字符数，按正例宏平均 | 脱敏替换、整块遗漏、前缀截断均不能仅凭source_id判满分；与“全部人工事实送达题数”分开 |
| Strict Answer Accuracy | 去首尾空白后的原串精确匹配；拒答题要求明确abstained | 全部14题和12道可回答题分开；失败留在分母 |
| Normalized Answer Accuracy | NFKC、大小写、空白、引号及句末标点的保守归一化后匹配 | 保留内部数字标点，1.5不等于15；不自动把完整句子、同义词、日期转换判对 |
| Semantic Answer Accuracy | 校准后的Judge判定全部必要事实正确、没有实质矛盾或虚构解释 | 没有Judge时为null，不用包含参考串代替语义正确性；当前采用改编rubric，非官方配置 |
| Abstention Accuracy | 无答案题声明拒答且Judge确认没有同时猜答案的题数/全部无答案题数 | 拒答声明率另报；未评分的拒答声明不能直接判对。Oracle空证据是弱控制，重点看有干扰上下文时能否拒答 |
| Unsupported Answer Rate | Judge认定有实质无依据断言的回答数/已完成支持性评分的非拒答回答数 | 即使猜中参考答案也可能unsupported；同时报告Judge覆盖率。未评分不计0 |
| 无答案非拒答率 | 无答案题中未拒答数/全部无答案题数 | 是官方负例上的风险代理，与所有正例回答的幻觉率不同 |
| 引用问题 | 未知source_id、缺少引用分别计数 | 合法引用不能证明事实被引用支持，需要支持性评分 |
| P50/P95、Token、费用 | 分别统计检索、Context、Reader、Judge、建索引和重启；保留n、失败、min/max | Judge耗时不算用户QA延迟；事后tokenizer计数不冒充provider usage；建索引不混进单次检索 |

没有分母的指标返回null。不同方法、重复运行不增加独立题数；不把四组14题写成56个独立问题。中断批次status=stopped，不生成完整Benchmark成绩。

旧版normalize会删除内部标点，使1.5与15发生碰撞；第三阶段使用独立的保守规范化器修正这个合同缺陷。历史评分函数和报告保留以便复现，不能将新旧评分器的结果不加说明地横比。

历史v1评分器有16个合成校准案例，覆盖完整句、同义、数字/日期、额外解释、数字近似错误、实体混淆、正确拒答、无依据猜中、算术、偏好rubric和部分答案。预期标签由本次工程工作编写，**人工复核状态仍为pending**，不能称已人工校准。文件为 `evals/datasets/judge_calibration_v1.json`。

收费计划预留16次Judge校准；人工确认标签后，当前v2要求16个有效结果、至少15/16整例标签完整匹配且关键错误为0；Judge不满足门禁，在锁定测试预测前停止。不能看到最终测试答案后改prompt、规则或参考答案。Reader与Judge同模型仍有相关偏差，需要对真实输出二次人工抽查。

## 检索与Context协议

| 组 | 实际能力 | 召回/输出预算 |
|---|---|---|
| A BM25 | 生产Repository/PG BM25，第二阶段最终SQL | 20/10 |
| B TF-IDF | corpus-only词袋cosine，额外控制组 | 20/10 |
| C Semantic | 本地Qwen3真实Embedding，评测内精确cosine | 20/10 |
| D Hybrid | BM25+真实Semantic，复用生产RRF和canonical授权 | 每路10，总20，输出10；权重1.15/1.0，k=60 |
| E Hybrid+Rerank | 真实Reranker条件未满足时不运行 | 不用TransparentReranker冒名替代；另行预注册 |

当前分支的治理存储/BM25在PG，向量接口在Mem0Adapter，本地部署配置默认Elasticsearch。没有发现pgvector扩展、vector列或`<=>`路径；TSVECTOR全文索引不等于pgvector。本阶段不增加pgvector或重写生产检索架构。精确cosine控制组不能证明生产ANN的召回、持久化、重建或吞吐。

真实Embedding模型为 `Qwen/Qwen3-Embedding-0.6B`，revision为 `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`；1024维、last-token pooling、L2、固定query instruction。query/document指令分开。运行设备、dtype和依赖由health校验，记录真实版本。批大小32，max_length=4096；客户端预先计数，超出时停止，不允许网关静默截断。缓存绑定正文、revision、维度、归一化、指令、输入类型和运行参数。

Reranker已查到的完整revision为 `e61197ed45024b0ed8a2d74b80b4d909f1255473`。已有HttpReranker/KimiReranker和本地yes/no logits接口，但未运行真实重排；不因模型名字存在就填实验组成绩。

四个Reader组固定为 Oracle、BM25、Semantic、Hybrid，使用同一个Reader。TF-IDF先测纯检索，不额外加入付费Reader。方法顺序用固定seed和题目ID打散。查询向量每题只生成一次，两个语义组共享；单独报告该成本，不能用缓存让Hybrid显得更快。

为避免BM25/cosine/RRF分数尺度混用，QA采用**评测内rank-only Context**：先走真实canonical授权，再按排序打包，4096个正文Token、12000 UTF-8字节，最多10块；先完整块，最后至多截取一个前缀，其余遗漏。所有组同策略，不看答案。它不是默认生产0.4相关性闸门的成绩；生产策略须另成组。

保存每块原长度、实际前缀、tokenizer SHA、实际Reader消息、遗漏原因及分阶段计时。Oracle也受同预算约束，覆盖不足时不能称证据充分。Oracle无答案题的空上下文表现不用于证明稳健拒答。

## Reader、Judge和费用闸门

候选Reader/Judge为硅基流动CN `deepseek-ai/DeepSeek-V3.2`；temperature=0、JSON输出、关闭thinking，另设置thinking_budget=128并计入最坏输出预留。公开服务列出了模型，但**未进行账户认证或收费可用性探测**；前2次授权后的smoke负责验证模型、非思维链、JSON与usage合同。

2026-11-07核对官方[价格页](https://www.siliconflow.cn/pricing)及[调价公告](https://api-docs.siliconflow.cn/docs/release-notes/overview)：输入4元/百万Token，输出6元/百万Token；不假设缓存折扣。[API文档](https://docs.siliconflow.cn/docs/api/chat-completions-post)说明max_tokens不包括思维链，所以不能只限制可见答案。非思维链合同异常会立即停止，保留已发生费用与预留。

| 用途 | 次数上限 | 每次输入/计费输出上限 | 最坏费用元 |
|---|---:|---|---:|
| 14题×4组Reader | 56 | 16384/896（答案768+预留128） | 3.971072 |
| 56次评分+16次校准 | 72 | 24576/640（JSON512+预留128） | 7.354368 |
| 可用性smoke | 2 | 16384/896 | 0.141824 |
| 合计 | 130 | 输入2719744、输出98048 | **11.467264** |

本轮拟定硬上限为**12元**，低于用户允许审议的20元，不是消费授权。Judge输入包括参考答案、完整证据及预测，单独留足预算，不能复用Reader的输入估算。真实Formation的24次调用本轮未排入，不隐含扩大请求数。

每次调用前持久化最坏token和金额预留；成功按usage结算，超时/异常可能已计费则保留预留并停止。请求数、输入/输出总量、单次输入界、输出、金额均有限制；45秒网络超时、重试0。缺usage、超usage、输出截断、模型不符或异常thinking停止。账本不能覆盖重开；一份批准只启动一次，异常中断不自动另开账本续费。密钥只从授权后的专用环境变量读取，不加载.env、不写日志。

客户端依据已核实的提供方价格/输出合同控制费用；执行前必须再核对价格。平台若不遵守合同或更换计费，客户端无法独立保证供应商账单，应停止并核对。服务只暴露模型alias而非不可变权重revision：保存实际响应model/fingerprint/日期，不能把公开tokenizer的HF revision冒充托管Reader权重版本。temperature=0也不保证逐字复现。

## 性能、兼容性和当前证据

`phase3-performance-v2`在固定环境完成每档40次热请求、6次PG/评测桥重启首次请求，100/400/801块合计138次，无超时。重启保留本轮独立匿名卷，结束删除该卷；未碰已有容器。重启首次不等于OS冷盘，page cache未清空。

热请求P95依次为12.37/27.89/83.33 ms；重启首次P95为872.77/892.77/973.52 ms。每组分布、SQL时间、11次SQL调用、建连/迁移及资源记录均保存。该布局与第二阶段多租户夹具不同，不能把更低数字宣传成又一次SQL优化。

`phase3-startup-attribution`保持同一数据库温热、只重建桥6次：首请求P50 819.87 ms，其中tokenizer初始化804.45 ms、SQL 3.55 ms；第二请求6.80 ms，tokenizer 0.06 ms。它定位的是探针的LazyLock首次初始化，不是证明数据库花了800 ms，也不是生产HTTP SLO。

Conflict兼容性只补与本任务有关的两项测试：真实隔离PG中的冲突写入/回滚，以及ApiError的HTTP409 JSON（CONFLICT、retryable=false）。均通过，JSON序列化测试不冒充完整身份认证HTTP路由验收。生产处理逻辑本阶段未改。唯一键保持原样；第二个待审写入没有来源残留、仍只有一个待审候选。调用方应先处理审核再重试，不自动按500重试；仓库外调用方和UI行为未验证。

第三阶段生产侧仅新增这项API回归测试；本地模型工具只增加health元数据，推理逻辑未重写；评测桥只补tokenizer计时。其余工作在evals。

## 交付状态和后续执行

固定协议/价格/模型在 `evals/configs/phase3_*.json`；校准案例和独立锁在datasets；准备、真实模型Runner、预算、分阶段评分、免费语义工程集、性能和开发集复核都有CLI及单元测试。复现命令见 [evals/README](../evals/README.md)。

执行前还需：明确收费授权；人工复核16个校准标签；确认账户可用性与最新价格；确认本地模型health匹配及运行时间足够。之后才执行14题四组Reader/Judge、Oracle、拒答/支持性分析，并生成真实模型最终报告。未经执行的表格保持空缺，禁止用规则Reader、模拟响应或旧自建集替代。

## 免费真实Embedding工程集结果（已实际运行）

首次权重下载遇到网络中断，限次续传后成功；权重SHA与HF LFS对象名一致，完整模型revision、Torch/Transformers、MPS/float32、L2和1024维都已核验。未加载真实Reranker、Reader或Judge。

| 方法 | R@1 | R@5 | MRR@10 | nDCG@10 | 检索P50/P95 ms（不含共享query编码） |
|---|---:|---:|---:|---:|---|
| bm25 | 0.6500 | 0.9000 | 0.8125 | 0.8306 | 53.42/60.46 |
| tfidf | 0.6500 | 0.9500 | 0.8417 | 0.8635 | 11.01/13.45 |
| semantic | 0.7750 | 1.0000 | 0.9250 | 0.9478 | 21.30/26.75 |
| hybrid_semantic | 0.7000 | 0.9500 | 0.8738 | 0.9007 | 61.51/70.94 |

工程集195文档、20正例+4负例，四组共96条逐题检索记录；额外一次就绪预热+31次本地Embedding HTTP请求，外部收费调用和费用均为0。文档建向量约6.54秒，共编码219个输入；单独query编码P50约99.11 ms，对两个语义组共用并单列，不把省略这一步的表中延迟称为完整在线延迟。
精确模型输入为文档5388 Token、query指令及EOS合计834 Token；左填充批量计算量另存padded_document_batch_tokens。网关usage=0为占位，不冒充实际usage。

真实模型检查发现原离线计数少了每个输入的EOS；已按实际HF tokenizer postprocessor修正计数与4096边界。推理本身一直包含EOS；工程集所有输入远低于上限，24个cached-vector排名重算完全一致，原质量结果未改。新14题计数修正为1745797 Token。

具体反例h04：“为不想听到车流的人预订客房应参考什么习惯？”正确块syn在Semantic排2，在Hybrid排7。它仍在混合候选里，失分来自固定1.15/1.0加权RRF把词法噪声抬高，而非正确候选没有召回。已独立重算RRF排名并保存贡献。**这是@5排序退化，不证明QA退化：正确块仍在top10，真实Reader还未运行。**没有为这个反例调权重。

四组对4道无答案查询都返回候选，不能从100% Recall@5推导拒答或低幻觉。20正例且数据由同一开发者编写，Semantic 100%不等于泛化100%，也不证明公开Benchmark成绩；新锁定14题尚未预测。

逐题、模型清单、缓存重算和退化例：`evals/reports/phase3-semantic-holdout/`。本地模型预热、权重哈希：`phase3-local-model-smoke.json`、`phase3-local-model-assets.json`。最终待审议计划：`phase3-ready-for-review/plan.json`。此前准备目录保留作计数/源码冻结的历史草案，不再用于审批。

费用账本按实际usage和未折扣公示价结算估算，字段usage_list_price_cost_cny不冒充供应商实际账单；缓存折扣及账单核对另报。

## 文件与验证入口

| 文件 | 目的 |
|---|---|
| evals/memory_eval/protocol3.py | 固定Prompt、保守规范化、分阶段指标、答案/支持性/拒答分母 |
| evals/memory_eval/locked3.py | 源SHA、全会话隔离抽样、锁seal、预测输入与标签分离 |
| evals/memory_eval/models3.py | 精确Tokenizer/EOS、Context预算、真实向量、缓存、请求和费用预留账本 |
| evals/memory_eval/phase3.py | assets/prepare/待授权models-run、版本绑定、实际输入/输出/失败和报告 |
| evals/memory_eval/semantic3.py、rescore3.py | 免费真实Embedding工程集及旧开发集阶段复核 |
| evals/memory_eval/performance3.py、evals/harness/src/main.rs | 冷/热夹具、独立卷清理、分词与SQL计时 |
| evals/configs/phase3_*.json、evals/requirements-phase3* | 真实模型/价格/参数/预算与可重建运行依赖 |
| evals/datasets/longmemeval_phase3_lock.json、judge_calibration_v1.json | 未预测独立子集、待人工复核的评分正反例 |
| evals/tests/test_phase3.py | 预算、审批、数值评分、证据、截断、EOS和真实向量接口合同测试 |
| apps/memory-service/src/api.rs | 只新增409错误JSON回归测试，业务处理未改 |
| tools/retrieval_models_server.py | health补实际运行参数/版本，不改变推理算法 |
| README、evals/README、相关docs | 命令、实验边界、价格及面试材料；旧报告保留 |

41项Python测试通过（含原13项），没有把跳过计作通过。另仅跑与本轮有关的1项API错误JSON测试和1项真实PG冲突事务测试，均通过。付费入口缺批准时在数据库、模型和密钥读取之前拒绝；证明保存在phase3-no-authorization.log。

完整保护与运行结果索引见evals/reports/phase3-validation.json和phase3-summary.json；最终消费审议只使用phase3-handoff/plan.json，之前所有准备目录均为源码/协议冻结前的草案。
