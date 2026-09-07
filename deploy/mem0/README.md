# Mem0 运行时内部说明

正式 B 模式固定使用 Mem0 commit
`b357a5a1b03c299ec8229c268e63cfac0f7c6566`。Mem0 负责从完整对话中提取、
更新、删除和检索长期记忆，Elasticsearch 保存向量；Mem0 PostgreSQL 保存服务
认证与配置状态。

## 对外边界

Agent 只调用 Memory Service：

- `GET /health/live`
- `GET /health/ready`
- `POST /v1/runtime/memory/retrieve`
- `POST /v1/runtime/memory/extract`
- `DELETE /v1/runtime/memory/{memory_id}`，需要 `memory:forget` 权限

本地 B 模式只把 Memory Service 映射到宿主机 `127.0.0.1:18081`。Mem0 与
已有 Elasticsearch 的网络入口由 Store 管理员维护；二者不能作为 Agent 的直接调用接口。

## 固定镜像

本地通过唯一构建入口生成固定版本的 Mem0 镜像：

```bash
./scripts/build-mem0-elasticsearch.sh
```

本地脚本与 ECS CI 共用 `deploy/mem0/Dockerfile`，原样打包固定的 Mem0 源码，
仅安装 Elasticsearch 客户端并附带本项目的配置/管理工具。不改写 Mem0 源文件，
不删除第三方包中的未使用插件，不采用启动猴子补丁。
本项目配置器默认且只接受 Elasticsearch，不再生成其他向量后端配置。

本地候选标签为 `lazytt-mem0-api-elasticsearch:b357a5a1-es-client-v1`。
`tests/ecs/check_mem0_source.py <镜像> <上游源码目录>` 逐文件核对上游 Python 源码摘要，
确保打包没有修改实现。构建成功仅证明制品生成，不代表当前服务器可以启动 ES。

## 当前兼容性断点

**当前固定版本的免补丁 REST 启动仍为 BLOCKED，不能发布到 ECS。**
上游 `server/main.py` 的默认向量后端固定为 pgvector，导入时立即初始化；
`server_state.py` 递归合并默认配置与已保存配置，会把旧数据库字段带进 Elasticsearch 配置。
库支持 Elasticsearch，不等于该 REST 启动入口支持通过环境变量选择它。
不要为绕过此限制启用旧后端、写入假配置或恢复源码补丁。

后续需选择并验证支持官方 ES 启动配置的 Mem0 版本/入口，再完成原生迁移、
权限、配置回读和 ES 业务测试。此前经过补丁的镜像测试结果不适用于这个原样源码候选。
已保留的 `tests/ecs/database_docker.py` 是部署前的验收门槛，不能删除失败检查来允许发布。

## 一次性配置器

`deploy/mem0/configure.py` 在 Mem0 和 Elasticsearch 就绪后执行一次，配置：

- Elasticsearch vector store；
- OpenAI-compatible embedding；
- OpenAI-compatible LLM；
- 向量维度与索引名称。

配置器必须同时收到完整的 embedding 与 LLM 专用变量。它不会输出 API Key，
成功后只打印模型名、维度、索引名和 provider。

完整的首次部署、健康检查和 Agent 调用示例见仓库根目录
[README.md](../../README.md)。
