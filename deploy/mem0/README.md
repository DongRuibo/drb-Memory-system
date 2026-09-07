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
Elasticsearch 不暴露宿主机端口，不能作为 Agent 的直接调用接口。

## 固定镜像

本地通过唯一构建入口生成固定版本的 Mem0 镜像：

```bash
./scripts/build-mem0-elasticsearch.sh
```

本地脚本与 ECS CI 共用 `deploy/mem0/Dockerfile`，安装 Elasticsearch 客户端，
并应用配置合并、数据库连接和配置持久化补丁。
构建阶段先通过 `prepare_vector_store.py` 清理不再支持的上游向量后端实现、注册和专用连接池依赖，
最终阶段只复制清理后的运行源码，不继承旧 Mem0 基础镜像。
PostgreSQL 驱动保留给认证与配置数据库；首次启动只提供认证配置入口，
收到完整 Elasticsearch 配置后才初始化 Memory。已保存的其他向量后端配置会拒绝启动，不能静默迁移数据。

本地镜像标签为 `lazytt-mem0-api-elasticsearch:b357a5a1-es-only-v1`，避免继续复用旧缓存。
ECS 仍以 GHCR digest 固定运行制品。代码变更不会自动更新现有容器或清理共享数据库。

当前固定上游版本仍需要定制构建：仅更换运行环境变量无法移除镜像内的后端源码与注册，
本项目还需要数据库连接及配置保存方面的兼容补丁。这里只构建一个最终 Mem0 镜像，
ECS 拉取并运行它。以后升级上游时重新核对补丁；源码不匹配会中止构建。

构建后的离线检查验证后端实现、注册及专用连接池均不存在，默认配置仅为 Elasticsearch，
空配置不初始化 Memory，旧后端配置不能启动或写入。禁用后端名称仅留在构建清理脚本和防回归测试中。

```bash
docker run --rm --network none -e MEM0_TELEMETRY=false --entrypoint python -i \
  lazytt-mem0-api-elasticsearch:b357a5a1-es-only-v1 < tests/ecs/check_mem0_image.py
```

`tests/ecs/database_docker.py <镜像>` 使用可自动清理的隔离测试数据库验证原生迁移、权限、
首次配置、API 脱敏、ES 向量写入/检索及重启回读，不调用模型 Provider，也不连接现有 Store。

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
