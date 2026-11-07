# 验证记录

最终完整运行：`2026-11-06-local-v3/`。本地独立复跑：`2026-11-07-local-repeat/`。`local-v1/local-v2` 是中断的失败运行，manifest 已标记 failed，不纳入最终汇总。

- 新增 Python 检查 11 项通过：规范评分、多相关 nDCG、重复事实/最大匹配、固定 Adapter 抽样、标签隔离、分块边界、防覆盖、超时不算正确拒答。
- 相关现有 Rust 单元测试 64 项通过。16 个原 PG 集成测试保持 ignore，未声称通过；本轮另外运行 11 个真实 PG 治理探针。其中 10 个验证指定能力，1 个验证已知同义改写去重缺口。
- 最终完整运行包含 480 条检索结果、24 条 Formation、66 条 QA。12 道公开题包括 10 道可回答题和 2 道拒答题。9767 个公开历史块中成功索引 9755 块，12 块拒绝、14 块脱敏；没有删除题目。
- 另一个全新临时数据库的复跑中，自建集四组、三轮的全部排名及 Recall/MRR/nDCG 一致，QA 预测和评分也一致；不要求延迟逐毫秒相同。
- `git diff --check`、新 Python 模块语法检查、UTF-8 检查通过；Rust bridge 使用独立锁文件离线编译通过。环境未提供 cargo-fmt/rustfmt，未声明格式工具检查通过。
- 开始时已有的 32 个修改/未跟踪文件逐字节保留。原 Cargo.toml/Cargo.lock 仍保持原有修改；本轮已有文件只追加 README，业务代码、迁移和部署配置没有修改。未提交、推送或合并分支。
- 所有本次带 drb-eval 名称/标签的临时容器已清理，原五个容器仍在运行。没有连接生产数据库，没有读取 `.env`，没有调用付费模型 API。

公开运行开始后，仅 CLI 增加了“output 已存在则提前拒绝”的保护；Runner、评分、基线、Adapter、Rust bridge 在该运行期间没有改变。完整报告的 manifest 保留启动时哈希，独立本地复跑使用最终 CLI，验证结果不受该保护逻辑影响。

复现检查：

```bash
python3 -m unittest discover -s evals/tests -v
CARGO_TARGET_DIR=/tmp/drb-memory-eval-target cargo test --locked --offline \
  -p memory-domain -p memory-application -p memory-postgres -p kimi-extractor --lib
python3 -m evals.memory_eval --output evals/runs/repeat-002 --seed 20261106 --repeats 3
git diff --check
```
