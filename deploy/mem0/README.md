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

先构建固定 Mem0 基础镜像和 Elasticsearch 扩展镜像：

```bash
./scripts/build-mem0-elasticsearch.sh
```

`deploy/mem0/Dockerfile.elasticsearch` 安装 Mem0 锁定版本对应的 Elasticsearch
客户端，并应用 `patch_server_state.py`。补丁只修正从 pgvector 切换到
Elasticsearch 时的配置合并行为。

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
