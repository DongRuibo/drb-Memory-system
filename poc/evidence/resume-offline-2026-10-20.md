# 2026-10-20 恢复准备验证回执

这是 Docker 启动前的阶段一记录。后续新增迁移、数据库和模型验证见
[本地运行验证回执](./resume-runtime-2026-10-20.md)。

基线：`2b48ba9`；恢复分支：`codex/resume-pgvector`。
本回执记录本轮新执行的检查，不引用历史模型成绩。恢复修改尚未提交。

| 检查 | 结果 | 证据边界 |
| --- | --- | --- |
| `cargo test --locked --offline --workspace -j 1` | 142 passed，20 ignored，0 failed | 使用本地缓存编译；需要本地 loopback 的模拟 HTTP 测试在允许本机端口的执行环境重跑通过 |
| `python3 -B -m unittest discover -s tests/resume -v` | 4 passed | 替身进程验证端口/CORS/启动顺序/禁用开关；模拟配置异常验证脱敏 |
| Web `npm ci --offline --ignore-scripts --no-audit --no-fund` | 完成，504 packages | 使用本地 npm 缓存；未执行安装生命周期脚本 |
| Web `npm test` | 构建成功，2 passed | SSR 渲染用例；不是浏览器业务 E2E |
| Web `npm run lint` | 通过 | 前端静态检查 |
| 修改的 shell 启动脚本 `bash -n` | 通过 | 语法检查 |
| `git diff --check` | 通过 | 补丁空白检查 |
| 工作区 Clippy | 仅放行已有 `manual_clamp` 告警后通过 | `--locked --offline --workspace --all-targets -j 1 -- -D warnings -A clippy::manual-clamp` |

Rust 使用本机 1.97.1 aarch64-apple-darwin 工具链。测试重跑及 Web 构建期间外部代理
指向本机不可用端口，localhost/127.0.0.1 除外；测试使用合成数据。
未下载/加载模型，未调用真实云 API。未使用原工作区的真实 `.env`。

无例外的 `clippy -- -D warnings` 未通过：基线已有的 Elasticsearch adapter 第 340 行
使用 `.max(100).min(10_000)`，触发 `manual_clamp`；已通过 `git show HEAD:...` 确认
它不是本轮引入。保留原代码，只在第二次检查中放行该规则。Rustfmt 对修改文件的检查
也提示 worker 中三处基线已有的格式差异，本轮未扩大为格式清理。

数据库 20 项 ignored 测试中包含本轮扩展的连续冲突/幂等和空租户 bootstrap 回归。
Docker Desktop daemon 不可用，因此这些测试及 SQL invariant 未执行。
真实 pgvector、Mem0、模型网关、完整 Web/Agent 链及语义评测也均未执行。

阶段一判定：离线准备通过；当时数据库和真实模型验收未完成。
