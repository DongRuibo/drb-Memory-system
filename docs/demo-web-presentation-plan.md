# Web 演示计划：让外行看懂 Agent 记忆

## 演示目标

观众最终理解三句话：

> 它从自然对话中形成受治理的长期记忆；类型说明“记住了什么”，范围决定“谁能看到”。
>
> 时间、结构化内容和旧版本都绑定到不可变 Memory Version，可以检查，而不是根据模型
> 的话术猜测。
>
> 每轮只把相关、当前且仍有权使用的版本交给模型；Forget 后立即停止使用。

页面保持常见大模型布局：左侧身份和对话，中间聊天，右侧 canonical 记忆档案。

## 会前准备

1. 启动 PostgreSQL、Mem0 和 Qwen3 Embedding/Rerank 网关。
2. 确认根目录 `.env` 权限为 `0600`，当前 shell 的 Kimi 配置有效。
3. 确认 8080 没有遗留 Memory Service，然后执行 `./scripts/demo-web-stack.sh`。
4. 打开 `http://localhost:3000`，确认“真实链路已就绪”。
5. 以 Alice 执行“重置当前用户”和“重置共享项目”，再分别切换 Bob、Carol 重置其
   当前用户数据。
6. 关闭含密钥、数据库密码和长日志的终端窗口。

启动脚本为本次本地进程生成临时 JWT secret。浏览器只有 HttpOnly Viewer Session，
不接收 Kimi Key、Memory JWT、数据库凭据或任意权限字段。

## 10 分钟主讲脚本

### 1. 自然形成四类记忆

保持 Alice，点击“1 自然形成记忆”。输入包含职业、Rust、项目权威数据库、带明确日期
的 Outbox 经历和回答偏好，不使用“请记住”口令。

指出：

- `preference`、`profile_fact`、`semantic_fact`、`episodic_summary` 是四种正式类型；
- user/project 是范围，不是额外类型；
- 项目事实进入 Project Scope，其余符合 Policy 的信息进入 User Scope；
- 模型只提 Candidate，Policy 与 PostgreSQL 事务决定是否成为正式版本。

### 2. 查看时间、结构和版本

点击“2 查看记忆结构”，展开右侧卡片：

- 经历摘要展示 2026 年 7 月 15 日的半开 Event Time、`Asia/Shanghai` 和精度；
- Structured Content 用类型专属白名单字段展示，原始 JSON 只在高级折叠区；
- canonical readable content 仍是权威正文；
- 当前版本、来源数和版本时间线均来自 canonical export。

### 3. 跨对话按需回忆

点击“3 跨对话回忆”。页面创建 Alice 的新 Chat Session。回答下方展开“查看版本证据”，
检查实际注入的 Memory Record ID 对应版本、范围、来源、Context Pack 和 Outcome
Receipt。强调没有把旧聊天整段伪装成长时记忆。

### 4. 按事件发生时间回忆

点击“4 按时间回忆”。查询“2026 年 7 月处理过什么工程问题”。回答应引用 Outbox 经历
及其解决办法。说明 Event Time 是被描述事件发生时间，不是消息保存时间、版本创建时间
或事实生效时间。

### 5. 项目事实生成新版本

点击“5 事实更新”。Alice 明确说明权威数据库从 PostgreSQL 改为 CockroachDB：

- 新值形成 v2 并成为当前 Head；
- v1 保留在审计时间线；
- 回答使用精确 v2；
- Correction 不盲目复制可能已经过时的旧 Structured Content，自动重新提取可为新版本
  形成新的受验证结构。

### 6. 精确 Forget 与 Erase 边界

点击“6 忘记与清除”。只 Forget 7 月 15 日的 Outbox 经历，职业、偏好和项目事实仍可用。
PostgreSQL Head 先阻断读取，Mem0 投影随后删除。右侧“物理清除”是不可恢复的 Erase，
不应与可审计的 Forget 混为一谈。

## 用户切换验证

1. 切到 Bob：Alice 的 Chat 和 User Scope 记忆消失；LazyTT 项目事实仍可见。
2. Bob 不显示“重置共享项目”，因为没有该演示权限。
3. 切到 Carol：LazyTT 项目事实不可见，只能访问外部沙盒项目。
4. 切回 Alice：Alice 的对话和个人记忆恢复。

身份切换会关闭旧 SSE、清空页面旧状态，并使用新的请求级 Active Principal。固定身份
只是本地演示 fixture；生产必须接外部身份和 membership authority。

## 常见问题

### 它会偷偷记密码吗？

不会。Secret 检测、类型白名单、Scope、置信度和 Policy 都在正式写入前；提取失败时
回答可以继续，但系统不能声称已经保存。

### 搜索到就一定会使用吗？

不会。Mem0 是可重建投影。每个命中都回到 PostgreSQL 检查主体、成员关系、用途、
当前版本、保留期、同意状态和 Forget/Tombstone。

### Alice 和 Bob 为什么能看到同一条项目事实？

两者当前都被服务端 allowlist 为同一项目成员；读取是请求 Scope、Principal membership、
permission、purpose 与记录 Scope 的交集。个人资料不会因此共享。

### 这已经是生产多用户登录吗？

不是。当前只完成 loopback 本地真实链路。生产仍缺 OIDC/JWKS、正式 membership
authority、撤权事件、TLS/网关、HA、灾备、容量压测和合规验收。

## 现场故障策略

- 页面未就绪：检查 8080、8090、18082、18888 和 55432，不要用静态数据伪装成功。
- `VersionMissing`：先确认迁移目录连续、`memory-postgres` build script 已追踪迁移文件，
  再重新构建服务。
- 切换用户被拒绝：等待当前 Turn 完成或先停止生成。
- Kimi 失败：展示真实终态错误；若使用 loopback 假模型，必须明确称为确定性备用模式。
