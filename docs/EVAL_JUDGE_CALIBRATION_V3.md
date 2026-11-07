# Judge Calibration V3最终小范围修正

本轮收费API调用0、费用0，不执行正式实验。v1、v2原JSON保留，v3等待用户最终授权。旧版本记录和报告不覆盖。

数据：`evals/datasets/judge_calibration_v3.json`。

SHA256：`0c3ca2749d095bacf8a44af97c0802b15280b2fbbbf5646c9914b6d634b410c4`。

唯一事实改动是preference_rubric_positive的context：

> Ada enjoys chess, prefers quiet indoor activities, and wants to avoid traffic noise and crowds.

保留true/true；preference_rubric_negative的问题、事实、回答和标签原样保留。全部16例参考答案、预测回答及预期标签均与v2一致。变化前后值和理由在v3的changes中；derived_from绑定v2原始SHA。

## 最终关键门禁

A表示semantic_accuracy，S表示supported。以下任一情况触发即停止，不能被15/16总一致率抵消。

| 案例 | 禁止的Judge判断 |
|---|---|
| decimal_error | A=true或S=true |
| unsupported_guess | A=true或S=true |
| lucky_guess | A=false或S=true |
| entity_swap | A=true或S=true |
| partial_answer | A=true或S=false |
| invented_explanation | A=true或S=true |
| correct_abstention | A=false或S不为null |
| unnecessary_abstention | A=true或S不为null |
| preference_rubric_negative | A=true或S=true |

共18条条件。两个not_null条件分别模拟true和false，共20组独立反例；每组只改一个Judge标签，其余15例完整匹配仍不能通过。模拟列表独立于数据中的critical_checks，避免漏配规则也让测试同步漏掉。

拒答的非null支持标签在parse_judge入口就会拒绝并停止；汇总门禁也能识别这类模拟输出。缺少supported字段、字符串null和整数等无效标签同样被拒绝，不静默转为null。

完整校准仍要求：收齐16个有效唯一ID结果、至少15例accurate/supported完整匹配、关键错误0。分母固定16，不对32个标签单独平均，不因缺失结果缩小分母。拒答null不是无幻觉证明。Reader/Judge输出格式、Prompt、检索和上下文策略未改。

## 预算及执行条件

56 Reader＋56正式Judge＋16校准Judge＋2可用性检查＝130次。按已封存的输入4/输出6元每百万Token配置，最坏估算11.467264元，硬上限12元。总输入2719744、计费输出预留98048 Token；重试0、总超时45秒、持久化预留和所有角色次数限制保留。

budget_key仍为`75cf9507780fa669528495783f6b5880bd62dc59a13075e3382c2ce18dd67679`。校准版本和代码变化不重置预算标识，不覆盖已有账本或claim。本轮没有创建真实收费claim或批准文件。

新执行计划位于evals/reports/phase3-calibration-v3，旧计划因文件/hash/源码变化失效。离线准备可进入授权后启动流程，但不代表账户访问、实时价格、实际非思维JSON合同或真实Judge校准已验证；这些必须在授权后按原计划前置检查。托管权重不可固定、Reader/Judge同模型、无答案题仅2题等既有边界不变。

## 文件与验证

| 文件 | 目的 |
|---|---|
| evals/datasets/judge_calibration_v3.json（新增） | 指定事实修正、18条关键规则、版本来源与逐项差异 |
| evals/configs/phase3_protocol.json | 更新版本并钉住v3路径、版本和SHA |
| evals/memory_eval/phase3.py | 切换v3，入口核验数据SHA及not_null规则 |
| evals/memory_eval/protocol3.py | 增量支持not_null门禁；Prompt不变 |
| evals/tests/test_calibration_v3.py（新增） | 9项回归，含20组关键错误模拟、缺失结果、null、版本及预算保护 |
| evals/tests/test_calibration_v2.py | 历史合同显式读取v2；当前入口检查更新为v3 |
| evals/README.md及当前清单提示 | 指向当前版本和封存计划，避免使用旧计划 |
| evals/reports/phase3-calibration-v3（新增） | 新计划、Prompt快照、模拟门禁记录、测试日志和保护校验 |

运行命令：

```bash
PYTHONPATH=/tmp/drb-phase3-deps python3 -m unittest discover -s evals/tests -v
PYTHONPATH=/tmp/drb-phase3-deps python3 -m evals.memory_eval.phase3 prepare \
  --assets /tmp/drb-phase3-assets \
  --longmemeval /tmp/drb-eval-public/longmemeval_s_cleaned.json \
  --output evals/runs/phase3-calibration-v3-001
```

本轮仅运行离线测试和prepare，不运行models-run。最终测试及原有32文件、v1/v2、锁定14题、历史报告、Prompt和预算标识的校验见新报告目录tests.log、validation.json。模拟Judge不等于真实模型成绩。
