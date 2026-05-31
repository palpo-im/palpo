# Palpo 代码审计与改进记录

本轮审计聚焦安全风险、未完成实现、死代码、部署示例可用性和配置默认值一致性。所有本报告列出的问题已在本轮收敛。

## 已完成

- [x] 修复 `openid_token_ttl` 默认值单位错误。配置文档说明默认值为 3600 秒，但代码返回 `60 * 60_000`，导致默认 OpenID token 有效期约 41 天。
- [x] 修复 appservice 请求失败日志泄露 `access_token` 的风险。发送给 appservice 的 URL 会携带 `access_token` 查询参数，失败日志原样输出 URL 会泄露 homeserver token。
- [x] 替换 `examples/*/appservices/*-registration.yaml` 中的固定 appservice token。示例里的固定 token 容易被复制到真实环境。
- [x] 修正 Docker 部署示例中 Postgres 初始化变量名：`POSTGRES_DATABASE` 不是官方镜像识别的变量，应使用 `POSTGRES_DB`。
- [x] 收紧基础 Docker Compose 中 Postgres 端口暴露范围，避免默认监听所有宿主机网卡。
- [x] 修复 Caddy 部署示例中错误的镜像地址 `docker.io/ghcr.io/...`。
- [x] 修复 HTTP client 连接池/超时配置未接入的问题，并将 `request_idle_per_host` 默认值与文档保持一致。
- [x] 修复未实现的管理命令返回“成功”的问题。`reload-config`、`reload-mods`、`restart`、`shutdown` 现在会明确返回未实现错误，避免误导管理员。
- [x] 修复 `config::reload` 空实现却返回 `Ok(())` 的问题。
- [x] 删除未接入的 `admin/debug` 死代码模块，并移除仍指向 debug 命令的注释和配置示例。
- [x] 将旧 debug 命令中可安全复用的能力接成 admin-only REST API，供管理界面调试 PDU、房间状态、签名、server key、依赖信息和 federation ping。
- [x] 为 `PALPO_` 环境变量增加 `__` 嵌套配置支持，例如 `PALPO_DB__URL` 可覆盖 `db.url`。
- [x] 为 Docker 部署和桥接示例增加 `.env.example`，并让 Compose 通过 `PALPO_POSTGRES_PASSWORD` 和 `PALPO_DB__URL` 使用同一份数据库密码。
- [x] 替换部署与桥接示例配置中的 `changeme`、`root`、`your_password` 数据库密码为环境变量或明确占位符。
- [x] 替换本地与桥接配置示例中的 `root` 数据库密码和固定 TURN secret，避免示例凭据被复制到真实环境。
- [x] 修复多个未实现路由返回 `200 OK` 的问题。federation query fallback、third-party invite exchange、3PID onbind、dehydrated-device 相关占位接口现在返回明确的 `M_UNRECOGNIZED`。
- [x] 为房间目录可见性变更补基础权限检查：普通用户必须在房间内且具备修改 canonical alias 状态事件的权限；开启 `lockdown_public_room_directory` 时，发布公开房间仅允许服务端管理员。
- [x] 补跑并通过全量 `cargo check --workspace -j 4`。
- [x] 使用 `docker compose config` 验证基础 Docker、Caddy、Traefik 和 Telegram 示例的 Compose 文件可解析。

## 后续专项

- Matrix 协议能力缺口仍需按功能专项继续实现，例如完整 dehydrated device 流程、3PID bind 处理、third-party invite exchange、room history visibility 细节、key signature 校验、邮件 pusher 等。本轮已避免未实现路径返回假成功，并关闭了可安全修复的安全/部署/死代码问题。
