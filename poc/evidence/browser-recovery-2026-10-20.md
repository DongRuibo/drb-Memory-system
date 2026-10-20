# 浏览器反馈修复与复验（2026-10-20）

本轮基于 `f38c107` 修复用户浏览器实测中发现的问题。原始浏览器报告没有全部通过；
本轮后端真实回归通过，但修复后的六步按钮尚未完成浏览器交互验收。

后续 TypeScript 修复及完整检索复评见 [实验收尾报告](final-recovery-2026-10-20.md)。
本文保留该修复阶段的检查结果，不以较晚结果覆盖历史失败。
最终核对另发现当时重启后的探针清理误遗忘了原共享项目事实；已恢复为新记录并收紧清理范围。
本文“原用户状态指纹一致”仅指当时的重启检查，不能扩展到随后清理，详细纠正见后续报告。

## 原始证据

用户提供的 [原始 JSON](browser-recovery-2026-10-20/before/browser-test.json) 和 13 张截图
已复制到 `browser-recovery-2026-10-20/before/`。JSON 内保留当时的临时绝对路径，
对应截图在归档目录中使用相同文件名。它们均是修复前证据，不能当作修复后截图。

原流程的整段介绍抽取、日期召回失败；分拆补录后通过的项目仍保留补测条件。
报告中的 CockroachDB v3 是测试记忆中的事实值，底层数据库仍为 PostgreSQL＋pgvector。

## 定位与最小修复

| 问题 | 证据与修复 |
|---|---|
| 整段介绍后没有记忆 | 独立基线三次调用有两次返回 `EXTRACTOR_INVALID_RESPONSE: extraction response was truncated`。OpenAI 兼容抽取请求未设置 DeepSeek 的 `thinking.type=disabled`；补齐该适配，保留 4096 completion token 上限、严格 JSON 校验，不接受截断内容。通用兼容服务不新增厂商字段，Kimi 请求契约不变。 |
| 完整介绍漏掉常用语言 | 首次修复后的真实聊天保存了职业，却漏掉同句中的 Rust。补充职业与主要使用语言应分别作为资料事实的抽取规则；“代码示例使用某语言”仍是场景偏好。保留该次失败证据，验收逐项核对 Rust，而非仅检查类型数量。 |
| 日期提问漏召回 | 发现适用性复核只收到经历正文，没有收到 canonical 授权候选的 `event_time`。将已有事件区间传入复核，明确按其时区比较日期／时段；不降低相关性阈值，不扩大授权。独立基线中的一次日期复测能通过，因此原问题不是稳定必现，也不能声称只靠旧请求必然复现。 |
| 六步入口缺失 | 原“开始六步演示”只发送介绍，不会将自由对话切换为 guided。现在会创建 guided 会话后发送介绍；侧栏增加始终可达的“六步演示”入口，保留已有会话和记忆。 |
| 将下午说成上午到下午 | 给回答模型增加由同一事件快照换算的 `event_time_local`，明确时区和右端不包含的区间边界，并禁止扩大时段。UTC 证据保留，实际输入快照也保存该本地表达。该措施经指定用例验证，不保证所有自由生成回答均无时间错误。 |

抽取策略版本改为 `openai-compatible-memory-extractor-v4-deepseek-json-budget`；
共享抽取提示变化也将 Kimi 策略标记为 `kimi-k3-memory-extractor-v10-profile-fact-coverage`。
复核策略版本改为 `answer-support-v2-event-time:<hash>`，保留历史策略证据。

## 验证结果

| 验证 | 结果与范围 |
|---|---|
| Rust 工作区 `cargo test --locked --offline --workspace -j 1` | 通过；需要专门数据库配置的 ignored 测试未在该命令中执行。真实 HTTP 回归实际使用 PostgreSQL。 |
| 针对变更的 Clippy | 通过；沿用原 Elasticsearch adapter 的 `manual_clamp` 基线豁免。 |
| Web lint、生产构建、现有渲染测试 | 通过，2 项渲染测试；不等于点击六步按钮的浏览器验收。 |
| `npx tsc --noEmit` | 未通过：现有 Cloudflare `cloudflare:workers`、`Fetcher`、`D1Database` 类型缺失。本轮没有改动 Cloudflare 部署配置。 |
| 原始整段输入与真实聊天回归 | **19/19**；三个独立合成用户原样整段抽取均保留职业、Rust、偏好和经历；新租户 Alice 通过 11 轮真实聊天核对四类记忆、跨会话回忆、日期／日／月召回、下午表述、错误日期和上午负例、版本历史、遗忘后日期与关键词查询。没有拆句或人工补录。 |
| 现有真实抽取集 | **22/22**；覆盖偏好／事实分类、时间、无记忆、恶意输入和敏感内容拒绝。首次运行是 21/22，差异来自下面的验收脚本问题，失败文件保留。 |
| 现有适用性诊断集 | **16/16**；通过原来的场景偏好和事实正负例，不是独立留出集。 |
| 原演示数据保留 | 重启前后 Alice/Bob 的记录 ID、头状态、当前版本、revision 指纹一致；没有重置或物理清除原用户数据。 |

幂等断言修正：原脚本要求整个抽取响应 JSON 完全相同，但 Outbox 会推进
`projection_status`、`provider_object_id`、`attempts`。诊断重放确认只有这三项响应字段变化。
现脚本仅排除这三项进度字段，仍严格比较 proposal、diagnostics、operation/candidate/record/version
身份、canonical 状态与 revision，并额外核对重放前后账本 records（包含版本、来源和事件）完全相同。
`generated_at` 是导出时间，不作为账本内容比较。这是修正验收条件，没有改变生产幂等逻辑。

## 证据与复现

结果位于 [after/](browser-recovery-2026-10-20/after/)：

- `browser-baseline.json`：截断失败基线。该临时探针未等待投影就绪，里面的即时空查询不作为日期故障根因证据。
- `date-baseline.json`：等待投影就绪后的一次成功日期基线，保留不稳定性证据。
- `browser-fix-live-v1.json`：发现 Rust 漏抽的中间失败。
- `browser-fix-live-v2.json`：最终 19 项真实聊天检查。
- `browser-fix-extraction-v1.json`、`browser-fix-extraction-v2.json`：幂等断言修正前后结果。
- `browser-fix-applicability-v1.json`：16 项适用性回归。
- `original-user-preservation.json`：原用户状态指纹核对。

新增驱动 `scripts/verify-browser-regressions.py` 必须运行在**全新、专用测试租户**；服务配置与
脚本参数的租户必须一致。它会调用 DeepSeek，通过 Memory API 和 Demo API 生成合成记忆、
新会话、事实更新与遗忘，不执行浏览器操作。可用默认端口，也可指定独立验证实例：

```bash
python3 scripts/verify-browser-regressions.py \
  --tenant <全新测试租户> \
  --memory-base http://127.0.0.1:19180 \
  --demo-base http://127.0.0.1:19190 \
  --output .run/checks/browser-regressions.json
```

只使用已有本机模型、镜像和依赖缓存，没有下载模型。本轮确实调用了获授权的 DeepSeek。
临时验证实例与原网页服务分开；原演示仍使用 `resume-deepseek-v3-20261020`，保留已有 Alice/Bob 数据。

## 尚未完成与结论边界

当前旧 Codex 对话仍有浏览器权限拒绝记录，本轮没有绕过该限制。
需要在用户已经能够访问的新对话中，刷新网页后点击侧栏“六步演示”，完整执行按钮流程，
保存修复后截图。此项完成前，不宣布 Web 交互全部通过。

本轮没有重测冻结的完整检索发布集或性能；历史正例额外注入问题没有因这些用例通过而消失。
测试不证明生产容量、多租户全覆盖、全副本物理擦除或云模型永久稳定。
