# Judge V4 第一阶段执行器：离线封存

本次仅完成执行器集成及离线验证，收费调用 **0**；没有读取模型 API Key，没有创建真实收费 Claim，没有运行正式 LongMemEval 14 题。当前分支为 `codex/memory-evaluation-system`，未合并 main。封存源基线为 `9a5a438452502f71a207118ab104da5da5aa2071`。

## 封存范围与版本

当前执行协议为 `judge-v4-stage1-executor-v1`，入口为 `evals.memory_eval.judge4_stage1`。它只有第一阶段的 prepare/run，不包含正式 Reader、检索、112 次实验入口。通过两个门禁后只写交接凭据，凭据明确 `formal_authorized=false`、`formal_requests_allowed=0`。

| 资产 | SHA256 / Seal |
| --- | --- |
| `evals/configs/judge_v4_stage1.json` 文件 | `ec4327b6962444e59fd49c7d3456277cfb36d2132f0ec52ea8c18825e6b3b232` |
| `evals/reports/judge-v4-stage1-sealed/plan.json` 的 canonical plan seal | `6d0380d443587a0cf34de8a2b2fc73e601385b3a745189d4f9e0755561064ded` |
| V4 Prompt canonical digest | `ba7ab1acc3aec7b379313b7b20ca61a2226c05feb40418c6fd43022f4eed6d05` |
| 原 16 例 `judge_calibration_v3.json` 文件 | `0c3ca2749d095bacf8a44af97c0802b15280b2fbbbf5646c9914b6d634b410c4` |
| 新 14 例 `judge_contract_v4_controls.json` 文件 | `82c3e51a279ccdacb41abc1dbdbcbe66b5a3af4cc568ad9b5c1cb1f9e9800f7c` |

文件 SHA 对原始字节计算；canonical digest/seal 使用现有 `protocol3.digest` 的 JSON 编码，不能用整个 plan 文件的字节 SHA 替代。当前代码、模型、完整输入、Token 上限、价格上界、旧账本摘要均绑定在 seal 内。源码或输入变化会使批准失效。

原 V4 离线提案 `judge_contract_v4.json`、V3 Prompt、首次真实运行及其账本均保留，不更新历史 hash 来伪装兼容。共享客户端发生本次必要修改后，旧 `judge4 prepare` 的源码 hash 校验会失败；当前使用独立的 stage1 配置和入口。旧运行源码可在基线提交还原。

## 32 次调用与费用上界

以下是封存价格假设，不是本轮实时询价或账户账单。沿用上一轮公示价：输入每百万 Token ¥4，输出每百万 Token ¥6，不假设缓存折扣。正式授权时必须重新核实价格；不一致则停止重新审议，不在原计划下提高费用。

| 顺序与角色 | 最大请求 | 单次输入上限 | 可见输出上限 | 含额外预留的计费输出上限 | 最坏费用 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1–2：API 可用性检查 `smoke` | 2 | 16,384 | 768 | 896 | ¥0.141824 |
| 3–18：原 V3 的 16 例、使用 V4 Judge | 16 | 8,192 | 512 | 640 | ¥0.585728 |
| 19–32：新增 V4 14 例对照 | 14 | 8,192 | 512 | 640 | ¥0.512512 |
| 第一阶段合计 | **32** | 总额 **278,528** | — | 总额 **20,992** | **¥1.240064** |

所有调用均使用 SiliconFlow CN 的 `deepseek-ai/DeepSeek-V3.2`、temperature=0、JSON 对象输出、`enable_thinking=false`、`thinking_budget=128`，总请求时限 45 秒，重试 0。提供方只给模型别名，不能固定不可变服务 revision。若模型名、截断状态、非思考模式、JSON/schema 或 usage 不满足合同，保留预留并停止。

原实验 18 次调用的 usage 公示价估算为 ¥0.035568，第一阶段全部按上限计算后累计 **¥1.275632**。这仍在累计 ¥12 限制内，旧授权剩余额度不构成本阶段授权。输入采用 UTF-8 字节加消息封装余量的保守 Token 界检查；32 份完整输入已封存，校准/对照最大界 4,413，小于 8,192，不截断。

## 实际实现与失败语义

`judge4_stage1.py` 复用现有 `claim_execution`、`Ledger`、`ChatClient`、V4 adapter 和校准评分器。执行顺序为：核对完整 plan 与当前封存配置 → 核对新的明确授权及两个数据集人工复核状态 → 校验旧 Claim 所指账本与已封存 receipt 完全一致 → 拒绝已存在的其他 V4 阶段 Claim → 才允许读取专用 Key → 原子建立第一阶段 Claim → 顺序执行 2/16/14。

真实 CLI 的 Claim 目录固定为 `evals/runs/paid-budget/`。第一阶段键为 `dd7ea2a8e7898a628c1f07aca435e32b075c56c51d2bbea5b5d3b48bfd8da0db`，不随输出目录、批准文件副本或进程启动变化。现有原子独占创建与 fsync 保证并发启动只有一个执行者；已有、空的或部分写入的 Claim 都拒绝续跑。进程中途退出也不能新建输出目录重开预算，须先核账并取得新方案授权。不自动恢复请求。

共享客户端的最小必要改动是：**先持久保存完整输入和原始响应，再验证输出与 schema，最后按有效 usage 结算**。原先先结算再解析，会对无效输出提前释放最坏预留。现在请求前预留持久落盘；网络失败、HTTP 错误、JSON/schema 错误、截断、思考输出或 usage 缺失/超限均停止，未知费用不释放预留。有效响应只是语义标签不匹配时，按已知 usage 正常结算，然后门禁停止；这不是失败退款。

每次调用保存 payload（无认证头）、输入 SHA、请求序号、提供方请求 ID、原始响应、usage、延迟、预留和结算。HTTP 错误正文与供应商 Trace ID 在同一个总超时内采集。传输未返回响应时 raw_response=null，如实保留失败而不捏造输出。每条预算事件 fsync，崩溃后账本为权威记录。共享客户端成功调用的价格、Prompt 和参数不变，仅增加记录和校验顺序；生产 Rust/API、Reader、Memory 和检索实现未改。

## 两个独立门禁

第一道必须有 16 个有效唯一结果，至少 15/16 整例的 accurate/supported 全部匹配，关键错误 0。第二道必须有 14 个有效唯一结果，14/14 全部匹配，关键错误 0。拒答必须显式 supported=null；null 不算无幻觉。缺失、重复、未知 ID、非法 schema 不能充数。

运行中一旦出现关键错误，或即使剩余全部答对也无法达到阈值，就立即停止。缺失及未尝试项仍留在固定分母中；不能只统计成功样本。第一道未过不调用第二组，任意一道未过不写通过交接凭据，正式质量指标为 null。两个数据集分别保存 results 和 summary，不合并成一个分数。

partial_answer 已出现在 V4 Prompt 示例中；其成功只能验证已知回归，不能作为独立泛化证据。新增 c01/c02 等 14 例问题、实体和数值未作为 Prompt 示例；本次没有增加示例或根据真实输出修改它们。但这些仍是同一工程工作流构造并经用户复核的合成对照，不是外部团队的盲测集。

## 五项边界确认

1. **联合 Judge 仍同时看到 reference_answers 与 evidence。** 还保留 question_type/unanswerable 等准确性所需信息；Prompt 只要求支持性先判断，不能宣称参考答案已经从 Support 信息流隔离。
2. **support_only_view 只是方案审查辅助数据。** 当前执行器不调用它，也没有独立 Support 模型。发往 API 的是明确白名单构建的联合输入；新增 14 例不会包含 expected 标签、annotated_asserted_claims、omitted_required_items、critical_checks。参考答案不是预期 Judge 标签。
3. **若再次出现 incomplete→unsupported，当前封存程序可直接停止，无需修改门禁。** 不能在本程序中无修改自动切换两阶段 Judge；后续需另建信息隔离适配、调用数与预算方案并取得新授权。不得继续微调到碰巧过关。
4. **14 例与 Prompt 的具体示例隔离，语义家族有意相同。** 输入 hash 和逐题 Trace 可追溯；本轮脚本夹具通过不证明真实模型能泛化。
5. **同工作区跨轮次/重启/输出目录可核账，不能重开同阶段预算。** 先核旧已结算 receipt，再绑定稳定阶段 Claim。此机制不是服务商钱包或跨机器全局限额；复制整个仓库/人为删除 Claim 不在允许操作范围，不能宣称可阻止这种人为绕过。

## 离线验证结果

全部 **119 项离线测试通过，0 跳过**，其中本轮新增 25 项。测试不读取 Key，不访问模型提供方；本地 HTTP 夹具仅访问 127.0.0.1。最初受限沙箱禁止回环端口，产生 1 项 PermissionError；该日志保留，未计通过。之后允许本机测试端口的完整 119 项运行通过，见封存 tests.log。语法编译和 git diff --check 通过。

| 持久化模拟场景 | 模拟 dispatch 数 | 结果 | 正式调用 |
| --- | ---: | --- | ---: |
| 完整成功路径 | 32 | 原 16/16、新 14/14 分别通过 | 0 |
| partial_answer 再次误判支持性 | 16 | 第一道立即停止；对照未运行 | 0 |
| c01 不完整但有据被误判 | 19 | 第一例新对照即停止 | 0 |
| c02 同家族另一独立用例误判 | 20 | 第二例新对照即停止 | 0 |
| 第三次请求传输失败 | 3 | 保留 1 笔最坏预留；无重试 | 0 |
| 第三次响应非法 JSON | 3 | 保存原始输出；保留 1 笔最坏预留 | 0 |

这些响应由按用例 ID 返回预定标签的模拟 API 提供，不进行语言理解，不使用关键词评分。16/16 和 14/14 只是执行器及门禁测试，**真实 V4 模型结果仍为 null**。模拟成本不是实际支出。

其他回归覆盖：角色/总请求、输入/输出 Token、费用边界；预算预留后子进程硬退出；两进程同时启动只能一个成功；新目录/批准副本不能重启；空 Claim 不删除；旧账本变化/未结算则拒绝；超时、HTTP429、非法 JSON、缺 usage、超限 usage、思考与截断保留预留；拒答 supported 非 null 不结算；主门禁一次普通不匹配允许 15/16，两次普通不匹配提前停止；旧 V3 关键错误回放仍失败。测试用 Claim 均在临时隔离目录，正式 Claim 未创建。

## 与 V3 的完整差异及文件范围

| 文件 | 本次目的 |
| --- | --- |
| 新 `evals/memory_eval/judge4_stage1.py` | 32 次封存/批准校验/复用执行器/两个门禁/完整 Trace/只交接不自动正式实验 |
| 改 `evals/memory_eval/models3.py` | 校验后结算、持久化完整输入输出、采集 HTTP 错误正文与请求 ID |
| 新 `evals/configs/judge_v4_stage1.json` | 独立协议、稳定阶段键、来源及运行源码 hash、预算上限 |
| 新 `evals/tests/test_judge4_stage1.py` | 25 项完整模拟、故障、并发与崩溃测试 |
| 改 `evals/tests/test_judge_contract_v4.py` | 验证原封存协议对共享代码漂移拒绝运行，不重写历史配置 |
| 改 `evals/README.md` | 当前免费 prepare 和未来仅授权后 run 的复现说明 |
| 新本报告及 `evals/reports/judge-v4-stage1-sealed/` | 协议/计划/完整差异、测试日志、逐题模拟 Trace 与保护核验 |

V3→V4 Prompt 差异沿用上轮封存 `prompt.diff`，本轮未改任何 Prompt。共享客户端完整差异见 `shared-client-v3-to-stage1.diff`，本轮代码/配置/测试差异见 `implementation.diff`。原 V1/V2/V3 数据、14 题锁定文件、RRF、Reader Prompt、V4 Prompt、全部旧 reports、原收费 Claim 与账本保持原字节；原有 32 个文件按历史快照核验，不加入本轮改动。

## 复现与未来执行

以下 prepare、测试与回放均免费；输出目录必须全新：

```bash
python3 -m evals.memory_eval.judge4_stage1 prepare --output /tmp/judge-v4-stage1-review-new
PYTHONPATH=/tmp/drb-phase3-deps python3 -m unittest discover -s evals/tests -v
python3 -m compileall -q evals/memory_eval evals/tests
git diff --check
```

未来仅在用户明确授权 **本 seal 的第一阶段 32 次**、复核价格后，才能创建批准记录，提供专用环境变量并运行：

```bash
# 本轮未创建有效真实批准文件，未执行此命令。
python3 -m evals.memory_eval.judge4_stage1 run \
  --plan evals/reports/judge-v4-stage1-sealed/plan.json \
  --approval <获得明确授权后创建的第一阶段批准记录.json> \
  --output evals/runs/judge-v4-stage1-real-001
```

批准须绑定 seal、stage_id、scope、两个数据集人工复核、32 次及各角色限制、¥1.240064、累计 ¥12、价格复核和 reviewer，并明确 formal_authorized=false。没有批准先拒绝，不读取 Key，不创建收费 Claim。不从 .env 自动加载凭据。

## 后续 112 次：独立授权方案，当前不能执行

只有真实的两个门禁都通过、人工审查逐题证据、核对第一阶段固定 Claim 与 ledger SHA 且不存在未结算预留后，才能另行封存 **56 Reader + 56 正式 Judge** 的协议并请求明确授权。固定 BM25/Semantic/Hybrid/Oracle 四组、14 题、原 Reader/评分规则/RRF/上下文策略，不能使用测试结果反调参数。新 seal 必须绑定原 14 题锁定 hash、当前 V4 adapter 和第一阶段账本 receipt。正式执行器尚需独立集成，不能用旧 V3 models-run 或本阶段交接凭据替代。

沿用封存价格和 Token 上限，112 次最坏费用 ¥9.691136；含原 ¥0.035568 和本阶段全部上限后累计 **¥10.966768 ≤ ¥12**。正式阶段稳定键预留为 `b220eb5ac063d22c106682c9ca9e8ddf51438ac19bd80082c4d24d6241c8bd9e`，现在没有创建。正式预算须用第一阶段真实已花/未结算预留核算，不得把未使用次数自动转移。额外可用性检查、重试、两阶段 Judge 或改变模型都不属于这 112 次，需要新的范围与预算确认。价格上调应缩小实验并重新封存，而非提高累计上限。

当前已具备第一阶段受授权执行的工程条件；真实价格/账户权限仍须执行前核实，真实 Judge 是否满足两个门禁仍未知。Reader 与 Judge 后续拟用同模型、模型服务 revision 不可固定、联合 Judge 的参考答案偏置、同源合成对照的局限仍须保留在最终报告中。
