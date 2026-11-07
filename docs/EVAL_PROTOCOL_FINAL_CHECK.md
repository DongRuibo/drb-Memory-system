# 收费前最终协议核验

本文为前一轮核验记录。当前校准集及Judge门禁已依用户复核更新为 [v2规则](EVAL_JUDGE_CALIBRATION_V2.md)，新计划位于evals/reports/phase3-calibration-v2；下文旧seal和pending说明按历史保留。

本轮收费调用0、真实模型推理调用0。仅执行本机HTTP慢响应夹具、临时文件和进程/线程测试。校准JSON及锁定14题均未修改，人工复核仍为pending。

## 人工复核材料

[16个完整案例表](JUDGE_CALIBRATION_REVIEW.md)逐字列出问题、参考、上下文、回答及现有标签，另给理由和歧义。理由是工程解释，不是替用户确认标签。
主要待决：bike歧义、10+7是否直接相加、主答案正确但额外事实错误是否整答判错、准确但无据与部分但有据的双维度标准、泛化偏好建议和具体活动断言边界。

## 实验矩阵

| 组 | 数据与召回预算 | Reader/Judge | 正式调用 |
|---|---|---|---|
| Oracle Evidence | 官方标记证据，经授权；与其它组相同10块/4096 Token/12000字节预算 | 同一DeepSeek-V3.2 | Reader14+Judge14 |
| BM25 | 单路20，最终10块 | 同上 | Reader14+Judge14 |
| Semantic | Qwen3真实Embedding，单路20，最终10块 | 同上 | Reader14+Judge14 |
| Hybrid | BM25 10+Semantic 10；固定1.15/1.0、RRF k=60，最终10块 | 同上 | Reader14+Judge14 |
| TF-IDF | 同语料、单路20/最终10块，额外纯检索控制 | 不运行付费Reader | 0 |
| 真实Rerank | 未排入本次计划；不拿透明重排替代 | 未运行 | 0 |

四个QA组共用同一批14题、Reader、Judge、各自固定的系统Prompt、temperature=0及JSON规则。方法顺序按固定seed打散。Oracle纳入对照，但若证据被授权过滤或预算省略，不能称该题Oracle证据充分；负例Oracle空上下文只是弱控制。

## 模型、Prompt与输出

- Reader与Judge提供方均为SiliconFlow CN，模型ID `deepseek-ai/DeepSeek-V3.2`。这是托管服务模型名，不是不可变权重快照；账户访问未验证。服务级同次对照可做，严格权重级逐字复现条件尚不具备。
- 本地Semantic为 `Qwen/Qwen3-Embedding-0.6B`，revision `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3`，1024维、L2、MPS/float32；这是eval内精确cosine，不是生产pgvector/ANN。
- 本地DeepSeek tokenizer revision `a7e62ac04ecb2c0a54d736dc46601c5606cf10a6`只说明计数资产，不证明托管Reader权重版本。真实usage、响应model/fingerprint在执行后记录，temperature=0也不承诺逐字确定。
- Reader固定输出 `{answer, abstained, citations}`；Judge固定输出 `{accurate, supported, reason}`，supported在拒答声明时为null。使用json_object；schema解析失败停止，不补发请求或修答案。
- Reader可见问题、提问日期和授权Context，不可见参考答案/has_answer；Judge与Oracle单独使用必要标注。
- 输入正文按4096个本地Tokenizer Token及12000 UTF-8字节限制，最多10块，至多截最后一块前缀。所有组同策略，不看金答案。API请求另按保守字节界和封装余量校验。

固定Prompt快照：`evals/reports/phase3-protocol-review-v2/prompts.md`。Reader digest `8607f3437b00718b55c0665179e3cacee4e028921286b8f83674f622933ae7e1`；Judge digest `21830100108600a3f9ce13b61106eb743fd8132cc4cb56db4c5f39fba19ffd11`。本轮没有更改它们。

## 评分和分母

| 项目 | 分母及处理 |
|---|---|
| 严格/保守规范化 | 主表报告12道可回答题；完整句、同义、数字/日期改写可能不匹配，交给语义维度另评 |
| 全14题的strict/normalized诊断 | 保留JSON，但负例按结构化拒答声明诊断，不是纯原串EM，不能冒充语义准确率 |
| LLM Judge准确率 | 完成批次的全部14题；准确性与证据支持分开，不用包含参考串就判对 |
| 拒答正确率 | 2道无答案题；声明拒答且Judge确认未同时给具体答案才算对 |
| Unsupported | 已完成支持性评分的非拒答回答数；声明拒答的支持性null，不当作无幻觉 |
| 检索指标 | 12正例；Evidence是官方has_answer块代理，不是人工事实标签 |
| Context覆盖 | 正例标记证据中实际仍相同的字符比例；截断/脱敏/遗漏不能仅凭source_id判满分 |

“答案正确但没有证据”和“回答不完整但所述有据”是不同错误。声明拒答却猜答案会在准确性/拒答维度失败，但支持性按当前schema不适用；这类输出应保留原文复核，不应把null解读成零幻觉。

## 锁定与统计边界

14题的题目、答案会话、全部历史会话与旧开发集交集为0；本轮只核验源SHA/锁seal和已有运行记录，不跑预测。没有使用这14题结果调融合权重、Prompt或评分规则。今后优化只在开发集做；已用于观察/调参的测试不能再宣称未见测试。
仅2道无答案题，**不足以估计稳定的拒答/幻觉率**：结果只能是0/2、1/2、2/2；四组输出不是8道独立无答案题。若要扩展，另立并预注册补充测试集，不改本锁、不合并成看起来更好的成绩。
同源公开数据的近似文本、生成风格和模型预训练污染未排除；ID和会话完全隔离不等于这些污染也被证明不存在。
16个校准案例也不足以覆盖知识更新、跨会话时间排序、冲突及提示注入；57项代码测试不等于模型语义Judge已经校准。

## 失败、截断与追溯

- 输入超过API界：发送前停止，不静默删历史；预登记的Context截断是正常策略，记录覆盖损失。
- 请求失败/45秒总时限到：重试0，保留最坏预留并停止。慢速持续返回字节不能延长总时限。当前真实HTTP只支持POSIX主线程顺序CLI。
- 输出finish_reason不是stop、超可见输出上限、异常思维文本/思维Token、模型不符、usage缺失或越界：记录原输出/usage并停止。可结算的实际usage按公示价记账，不能结算则保留预留。
- 未完成批次status=stopped，正式quality_metrics为null；报告完成覆盖和失败，不从成功题抽取一个“完整准确率”，不把失败或未执行题当通过。
- `traces/<question_id>/<method>.json`保存实际输入、Context、截断、引用和预测；calls.jsonl有请求序号、usage和费用；failure.json关联题目ID、方法和最后请求。

## 预算复核与必要修复

官方[价格页](https://www.siliconflow.cn/pricing)和[公告](https://api-docs.siliconflow.cn/docs/release-notes/overview)再次核实：DeepSeek-V3.2输入4元、输出6元/百万Token；缓存命中0.4元不用于降低预算。API文档表明completion_tokens包含reasoning，不能再重复加计；max_tokens仅限制可见输出，因此另预留128个思维Token，并禁止实际thinking。

| 角色 | 硬次数 | 输入上限/次 | 可见输出/次 | 计费输出预留/次 | 公示价最坏元 |
|---|---:|---:|---:|---:|---:|
| Reader | 56 | 16384 | 768 | 896 | 3.971072 |
| 正式Judge | 56 | 24576 | 512 | 640 | 5.720064 |
| 校准Judge | 16 | 24576 | 512 | 640 | 1.634304 |
| 可用性smoke | 2 | 16384 | 768 | 896 | 0.141824 |
| 合计 | 130 | 输入总2719744 | 可见总81408 | 计费输出总98048 | **11.467264** |

硬上限12元，余量0.532736元不是额外请求授权。达到次数、角色次数、Token或费用上限即停止；报价与执行共用同一价格配置，入口拒绝价格证据不一致和12元以上的配置。金额为按公示价与usage的保守估算，不冒充供应商账单。

| 发现的问题 | 本轮修复 |
|---|---|
| 一次性标记跟批准路径走，复制批准/换输出目录可重开预算 | 同一锁定集+Reader的稳定budget_key；规范位置原子创建持久claim，复制路径或改代码不重置 |
| 账本内存计数没有跨线程/进程保护 | 明确禁并发；记录创建者PID/线程，拒绝跨线程、fork和第二个未结算请求 |
| 角色次数只有总量限制 | Reader56、Judge56、calibration16、smoke2分别硬限制，总130 |
| 结算在持久化之前释放额度 | 预留先fsync才发送；结算先fsync才释放；磁盘失败停止并保留最坏预留 |
| 同一结算可能被重复使用 | 只允许当前pending结算一次，重复/未知结算拒绝 |
| quote写死4/6，未来价格配置可能不同步 | quote/ledger读取同一协议价格，并校验官方价格证据配置 |
| 45秒仅socket阻塞时间，不是总时限 | POSIX总时限中断，慢滴流测试验证，超时不自动重试 |

重启策略是**拒绝自动续跑**，不是恢复一个清零的新账本：已有calls.jsonl不可覆盖，已有实验claim即使为空/进程已退出也拒绝新目录启动。需要先核对账本与未决请求，再人工处理；不提供删除claim绕过上限的自动命令。
该保护范围是同一工作区的本实验Runner，不是跨机器分布式预算，也不约束其它持Key程序。供应商需遵守所公示的价格/Token合同；应用侧无法替不正确计费作无条件担保，平台金额额度若可用可作为额外保护，当前未验证，不宣称已设置。

## 验证、文件与开跑条件

57项离线测试通过、0跳过：包含原41项及16项本轮预算回归。第一次受限沙箱不允许本地端口监听的一项失败单列保留；最终运行允许本机随机端口，所有模型传输为模拟，无收费API或真实Embedding推理。未重跑无关Rust/PG模块。

| 文件 | 本轮目的 |
|---|---|
| evals/memory_eval/models3.py | 预算所有者、pending、角色次数、持久化顺序、总HTTP时限与思维usage检查 |
| evals/memory_eval/phase3.py | 固定实验claim、统一报价、12元入口限制、正确分母展示及新plan |
| evals/configs/phase3_protocol.json、phase3_prices.json | 仅补角色预算及价格复核元数据，检索和Prompt参数不改 |
| evals/tests/test_budget_audit.py、test_phase3.py | 16项新安全回归与既有审批fixture适配 |
| docs/JUDGE_CALIBRATION_REVIEW.md、本报告、evals/README.md | 人审清单、歧义、边界与复现入口 |

本轮新计划：`evals/reports/phase3-protocol-review-v2/plan.json`；seal `2adc9a65cc7f6190d58407d6948cc8cc553cb42a7bafca8a3db907d67712edb2`。旧handoff不覆盖，但源码/协议hash变化使其不能作为当前执行计划。

**结论：离线软件闸门已具备受控试验条件；尚不能直接收费开跑。** 仍需你逐项复核标签并明确消费授权；授权后先验证账户访问、JSON/usage和非思维链合同，再做至少15/16双标签一致的Judge校准。托管权重无法不可变锁定和负例数量不足的边界，需要你接受为小规模服务级探索，否则应另立更严格方案。

审批字段只接受布尔true；字符串pending、数字1或空白reviewer均不算人工复核完成。原校准JSON继续pending，不自动填通过。
