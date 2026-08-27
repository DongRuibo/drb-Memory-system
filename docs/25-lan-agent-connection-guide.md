# LazyTT 记忆服务局域网接入指南

本文给需要从另一台电脑调用 LazyTT Memory Service 的 Agent 开发者使用。

当前测试环境：

| 项目 | 当前值 |
| --- | --- |
| 网络 | 两台电脑连接同一个 WiFi |
| 服务端电脑 | 部署并运行 LazyTT Memory Service |
| 服务端局域网 IP | `192.168.31.113`（每次测试前需要重新确认） |
| 服务端 SSH 用户 | `drb` |
| Memory Service 本机地址 | `http://127.0.0.1:8080` |
| 对方电脑使用的地址 | `http://127.0.0.1:28080` |

## 1. 连接方式

Memory Service 不直接开放到局域网，而是继续只监听服务端电脑的本机地址。对方通过
SSH 加密隧道访问：

```text
对方 Agent
  |
  | http://127.0.0.1:28080
  v
对方电脑上的 SSH 隧道
  |
  | 同一 WiFi 内的加密连接
  v
192.168.31.113:22
  |
  v
服务端 http://127.0.0.1:8080
  |
  v
LazyTT Memory Service
```

这样不需要把 Memory Service、PostgreSQL 或 Mem0 直接暴露到局域网，也不需要修改
Docker Compose 的端口绑定。

## 2. 双方需要准备什么

服务端负责人需要：

- 启动 LazyTT Memory Service；
- 开启 macOS“远程登录”；
- 接收并安装对方的 SSH **公钥**；
- 在需要调用受保护 API 时提供受限、短期的 Memory JWT。

对方 Agent 开发者需要：

- 一台与服务端连接同一 WiFi 的电脑；
- 系统自带的 `ssh` 和 `curl`；
- 生成自己的 SSH 密钥；
- 保持 SSH 隧道运行；
- 在自己的 Agent 后端配置 Memory Service 地址和短期 JWT。

## 3. 对方先生成 SSH 密钥

在对方电脑执行：

```bash
ssh-keygen \
  -t ed25519 \
  -f ~/.ssh/lazytt_memory_test \
  -C lazytt-memory-test
```

它会生成：

```text
~/.ssh/lazytt_memory_test       私钥，不能发送给任何人
~/.ssh/lazytt_memory_test.pub   公钥，可以发送给服务端负责人
```

查看公钥：

```bash
cat ~/.ssh/lazytt_memory_test.pub
```

只把以 `ssh-ed25519` 开头的这一行公钥发给服务端负责人。不要发送没有 `.pub` 后缀的
私钥文件。

## 4. 服务端负责人准备服务

### 4.1 确认当前地址

服务端电脑执行：

```bash
ipconfig getifaddr en0
whoami
```

当前记录是：

```text
IP：192.168.31.113
用户：drb
```

局域网 IP 可能在重连 WiFi 后变化。如果第一条命令返回了不同地址，应把新地址通知
对方，并在后续 SSH 命令中使用新地址。

### 4.2 开启远程登录

打开：

```text
系统设置 -> 通用 -> 共享 -> 远程登录
```

启用“远程登录”，并确认用户 `drb` 在允许访问的用户列表中。

### 4.3 安装对方的 SSH 公钥

服务端电脑执行：

```bash
mkdir -p ~/.ssh
chmod 700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
nano ~/.ssh/authorized_keys
```

把对方发来的 `ssh-ed25519 ...` 公钥添加到新的一行并保存。不要要求对方提供私钥，
也不要把服务端的 macOS 登录密码发给对方。

### 4.4 启动 LazyTT 后端

服务端电脑执行：

```bash
cd /Users/drb/code/lazytt-memory-system
./scripts/backend-stack.sh cloud up
```

确认服务正常：

```bash
curl http://127.0.0.1:8080/health/ready
```

只有本机健康检查成功后，才继续测试远程连接。

## 5. 对方建立 SSH 隧道

在对方电脑执行：

```bash
ssh \
  -i ~/.ssh/lazytt_memory_test \
  -N \
  -L 28080:127.0.0.1:8080 \
  drb@192.168.31.113
```

说明：

- `28080` 是对方电脑使用的本地端口；
- `127.0.0.1:8080` 是服务端电脑上的 Memory Service；
- `drb@192.168.31.113` 是当前服务端 SSH 地址；
- 命令正常运行时通常没有输出；
- 这个终端需要保持打开，关闭终端或按 `Ctrl+C` 会断开隧道。

第一次连接可能出现主机指纹确认。应先向服务端负责人确认 IP 和设备无误，再输入
`yes`。如果 IP 已变化，不要盲目接受新的指纹。

## 6. 对方验证网络连接

保持 SSH 隧道终端不动，另外打开一个终端执行：

```bash
curl http://127.0.0.1:28080/health/ready
```

健康检查成功表示：

- 两台电脑之间的 WiFi 网络可达；
- SSH 登录与公钥认证正常；
- SSH 端口转发正常；
- 服务端 Memory Service 正在运行。

健康检查不代表受保护的记忆 API 已经可以调用。记忆读写还需要下一节的短期 JWT。

## 7. 对方 Agent 的配置

对方 Agent 应在自己的**服务端环境变量**中配置：

```dotenv
MEMORY_BASE_URL=http://127.0.0.1:28080
MEMORY_BEARER_TOKEN=<REDACTED>
```

注意：

- Agent 必须访问 `127.0.0.1:28080`，不要直接访问 `192.168.31.113:8080`；
- Token 只能保存在 Agent 后端，不能进入浏览器或模型提示词；
- 不要把真实 Token 写进 Git、聊天记录、截图或日志；
- Token 过期后，应从受信任的签发方取得新 Token。

## 8. Agent 的最小记忆调用流程

一次正常对话建议按下面的顺序调用：

```text
用户发送消息
  |
  v
POST /v1/memory/extractions
提取并提交长期记忆候选
  |
  v
POST /v1/memory/context-packs
为当前 Task/Attempt 编译记忆清单
  |
  v
POST /v1/memory/context-packs/{context_pack_id}/consume
再次验证权限并取得允许使用的记忆正文
  |
  v
把返回内容作为 untrusted_memory_data 交给对方自己的回答模型
  |
  v
可选：POST /v1/memory/context-packs/{context_pack_id}/outcomes
记录本轮记忆使用结果
```

不要让 Agent 直接连接 PostgreSQL 或 Mem0，也不要直接把 `/search` 的原始命中结果放入
模型。正式回答链路应使用 Context Pack 的 Compile + Consume。

完整接口定义参见 LazyTT 后端仓库中的：

```text
docs/api/openapi.yaml
docs/15-agent-runtime-integration.md
```

## 9. JWT 的当前限制

当前 Docker 后端使用短期 HS256 JWT。Token 应至少绑定：

- `tenant_id`；
- `user_id`；
- 当前 Agent 或 Project 范围；
- 实际需要的 Memory permission；
- 最长一小时的有效期。

当前仓库还没有面向外部 Agent 的通用 Token 签发服务。因此当前测试分为两个阶段：

1. 没有 Token：只能完成 SSH 隧道和 `/health/ready` 网络验证；
2. 获得受限短期 Token：才能调用记忆提取、Context Pack、Forget 等受保护 API。

不能通过共享 `MEMORY_JWT_HS256_SECRET` 解决这个问题。签名 Secret 泄露后，持有者可以
伪造其他用户或 Tenant 的 Token。

## 10. 停止连接

对方停止 SSH 隧道：

```text
在运行 ssh -N -L 的终端按 Ctrl+C
```

服务端停止 Memory Service 并保留数据：

```bash
cd /Users/drb/code/lazytt-memory-system
./scripts/backend-stack.sh cloud down
```

不要执行带 `--volumes` 的关闭命令，否则会删除本地数据库和模型数据卷。

## 11. 常见问题

### SSH 端口无法连接

对方执行：

```bash
nc -vz 192.168.31.113 22
```

如果失败，双方检查：

- 两台电脑是否连接同一个普通 WiFi，而不是访客 WiFi；
- 服务端 macOS“远程登录”是否开启；
- 用户 `drb` 是否允许远程登录；
- 服务端防火墙是否允许远程登录；
- 服务端电脑是否休眠；
- 服务端 IP 是否仍然是 `192.168.31.113`；
- WiFi 路由器是否开启了客户端隔离。

### SSH 成功，但健康检查失败

服务端执行：

```bash
curl http://127.0.0.1:8080/health/ready
./scripts/backend-stack.sh cloud ps
```

如果本机健康检查失败，应先修复后端启动问题，而不是修改 SSH 配置。

### 健康检查成功，但记忆接口返回 401

说明网络已经正常，但请求没有携带有效 JWT，或 Token 已经过期。应联系服务端负责人
取得新的受限短期 Token。不要索要或共享 JWT 签名 Secret。

### 对方电脑的 28080 端口已被占用

可以换一个空闲端口，例如 `38080`：

```bash
ssh \
  -i ~/.ssh/lazytt_memory_test \
  -N \
  -L 38080:127.0.0.1:8080 \
  drb@192.168.31.113
```

同时把对方 Agent 的地址改为：

```dotenv
MEMORY_BASE_URL=http://127.0.0.1:38080
```

## 12. 严禁发送的内容

双方都不得通过聊天、邮件、截图或 Git 发送：

- SSH 私钥；
- macOS 登录密码；
- `.env` 文件；
- Kimi、SiliconFlow 或其他模型 API Key；
- PostgreSQL、Mem0 密码；
- `MEMORY_JWT_HS256_SECRET`；
- 长期有效或权限过大的 JWT；
- 包含真实用户记忆正文的调试日志。

可以发送：

- SSH 公钥；
- 当前局域网 IP 和 SSH 用户名；
- 不含 Secret 的 Memory Service 地址；
- 有限期、最小权限的短期 JWT；
- OpenAPI 文档和错误码；
- 已脱敏的健康检查结果。
