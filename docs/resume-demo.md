# PostgreSQL＋pgvector 简历项目演示

本分支基于 `2b48ba9` 恢复。PostgreSQL 是权威账本；Mem0＋pgvector 是检索投影；
本地 Qwen3 Embedding / Reranker 提供向量和候选排序；DeepSeek 负责抽取、回答及可选的
答案适用性复核。治理、授权、版本、冲突审核、Outbox 和遗忘由 Rust 服务实施。

## 当前验收范围

已通过真实合成聊天的跨会话回忆、修正、冲突拒读、人工审核、用户隔离和遗忘验证，
以及真实模型的抽取与 Episode 整理。具体指标、失败尝试和限制见
[本轮验收报告](../poc/evidence/deepseek-recovery-2026-10-20.md)。

浏览器自动访问本机页面曾被来源授权检查拒绝。Web 服务启动可用，但没有据此声称
浏览器交互、视觉布局、截图或录像验收完成。API 结果也不等于界面验收。

2026-10-20 用户在新 Codex 对话完成等价消息实测，发现整段抽取、日期召回和引导入口问题。
修复及真实回归结果见 [浏览器反馈修复报告](../poc/evidence/browser-recovery-2026-10-20.md)。
修复后侧栏有“六步演示”入口；完整按钮交互与新截图仍待浏览器复验，旧截图不作为修复后证据。

当前 TypeScript 检查已通过，固定集与预先冻结的新场景集也已重跑。
最新结论及未通过项见 [实验收尾报告](../poc/evidence/final-recovery-2026-10-20.md)，
网页操作按 [修复后复验清单](browser-final-checklist.md) 记录，不把 API 成功当作按钮点验。

## 本机启动

依赖：Docker Desktop、缓存镜像和完整的现有 Hugging Face 权重、Rust/Cargo、Node/npm，
以及已安装的 `apps/demo-web/node_modules`。命令禁止拉取镜像或下载模型；依赖缺失时先停止。
原部署使用的端口、数据卷和 `.env` 不作为运行配置直接复用。

恢复工作区已有一份被忽略的私有 `.env`，保存隔离数据库、Mem0 和 JWT 凭据。
首次创建方式见 [恢复说明](resume-recovery.md)，不要用原工作区 `.env` 覆盖它。
DeepSeek 密钥通过单独文件读取；该文件不被写入、不被 shell 执行，也不整体导入运行环境。
支持 `DEEPSEEK_API_KEY`／`DEEPSEEK_MODEL`，兼容现有 `apikey`／`model_name`。

```bash
cd /Users/drb/code/lazytt-memory-system/.run/resume-pgvector
export RESUME_HF_HUB_CACHE=/Users/drb/.cache/huggingface/hub
docker compose -p lazytt-resume-pgvector \
  -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  -f deploy/local/resume-semantic.compose.yaml \
  up -d --wait --pull never --no-build postgres mem0 contract-embedder contract-gateway
cargo build --locked --offline -p memory-service -p demo-web-api -p retrieval-eval
python3 scripts/resume-demo.py \
  --credentials-file /Users/drb/code/lazytt-memory-system/.env \
  --tenant resume-demo-20261020
```

启动器检查端口占用，依次等待 Memory API、Demo API 和 Web 就绪。打开
`http://127.0.0.1:13000`，Memory API 为 `19080`，Demo API 为 `19090`。
不要把这些带演示身份切换器的端口暴露到公网。

密钥仅通过后端进程环境传递；前端只获得公开 API 地址。启动日志位于
`.run/resume-live/`，新建文件权限为 0600，禁止整份公开上传。
`Ctrl-C` 停止启动器拥有的三个进程，不停止 Docker，也不删除数据。

## 可复现的验收

服务运行时另开终端，进入同一恢复工作区。以下命令会调用 DeepSeek，并只生成合成数据。
每组输出使用新的路径，不覆盖历史证据。完整聊天验收要求该租户的演示用户还没有旧记录，
因此重复验收应重启启动器并使用新的 `--tenant`，下面的租户参数也一起修改。

```bash
python3 scripts/verify-resume-chat.py --tenant resume-demo-20261020 \
  --output .run/checks/chat.json
python3 scripts/eval-resume-live.py --tenant resume-demo-20261020 \
  --output .run/checks/extraction.json
python3 scripts/verify-resume-episode.py --tenant resume-demo-20261020 \
  --output .run/checks/episode.json
python3 scripts/verify-resume-applicability.py --tenant resume-demo-20261020 \
  --output .run/checks/applicability.json
python3 scripts/measure-resume-retrieval.py --tenant resume-demo-20261020 \
  --output .run/checks/latency.json
```

这些驱动验证真实 HTTP 边界，不读取真实聊天材料；输出不包含 API Key 或 JWT。
抽取、Episode、适用性和性能驱动使用独立合成用户／项目，结束后遗忘测试记忆。
争议记录必须先审核再遗忘；这是现有状态契约，不会静默跳过清理。
Episode 在无未覆盖轮次时拒绝新整理请求，预期 HTTP 409；不把它当作重复创建成功。

完整检索集仍使用 `scripts/eval-retrieval-v2.sh`，JWT、active generation 和新输出目录的设置
见恢复说明。不要用遗忘后的旧数据直接查询并把空结果当作负例通过。

## 适用性复核与代价

`MEMORY_SUPPORT_ENABLED` 默认关闭；本演示启动器显式启用，并通过单独的
`MEMORY_SUPPORT_API_KEY` 和 `MEMORY_SUPPORT_MODEL` 配置 DeepSeek。
它只接收已授权的候选，不增加可见范围、不代替 canonical 回查；不执行候选文本中的指令。
事实需要能提供查询所需信息，场景偏好需要适用；global 偏好仍按明确的通用政策保留。

复核可以恢复被小模型低分排除的场景偏好，也会排除高分却缺少回答依据的事实。
最终非 global 适用性分数来自复核模型；Qwen 分数参与候选判断与同分排序。
新排序身份同时绑定 primary 策略和复核模型，用固定长度哈希满足回执和 Context Pack 的
128 字符约束。复核网络失败、截断、未知／重复 ID 或无效分数会拒绝记忆检索，
聊天端按既有逻辑退化为无记忆回答，并记录降级证据。

它增加云请求、网络延迟和供应商依赖；DeepSeek 的模型名称是供应商别名，本轮没有
固定云模型权重 revision。不能把其效果宣称为本地 Qwen 单独达到。

## 停止隔离依赖

```bash
RESUME_HF_HUB_CACHE=/Users/drb/.cache/huggingface/hub \
docker compose -p lazytt-resume-pgvector \
  -f compose.yaml -f deploy/local/resume-contract.compose.yaml \
  -f deploy/local/resume-semantic.compose.yaml stop --timeout 10
```

保留数据卷及本机缓存，不使用 `down -v`。当前结果只覆盖单机合成验证，不能当作生产
容量、高可用、全副本物理擦除或无人工监督运行的证明。
