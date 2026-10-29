# Hagency 应用退役

[English](HAGENCY_MIGRATION.md)

Palpo 保留 Matrix 客户端/服务器及联邦协议、房间、事件、App Service 和
homeserver 管理 API。本仓库不再实现 Hagency fleet 安装、配对、coordinator/项目
审批、资源目录和预算、Agent 生命周期、Inbox 通知及运行端 relay。
`palpo-hagency-contract`、`palpo-operations` 中的 Hagency 业务和旧 Node 迁移夹具
已移除。

普通账号注册审批仍保留在独立的[账号审批服务](../crates/operations/README.md)，
包括申请/回执接口、审批 worker 和受服务授权约束的原生管理会话。通用 App Service
注册、启停/删除、回调 URL compare-and-set、身份管理和可嵌入的 `MatrixServer`
库继续保留。Matrix 设备断言校验、拒收事务重试、凭据脱敏和成员资料同步修复也保留。

## 既有部署

移除源码不会停止已部署服务，也不会删除 SQLite/PostgreSQL 数据、账号、房间、
凭据或待处理申请。仍需要旧工作流时，请锁定原有可执行文件/镜像，并在替换前备份
私有数据库和配置。

精简后的账号服务保留现有注册审批状态、服务器/密钥绑定和 writer 隔离。
历史 JSON 扩展字段及表继续保存，但已退役的工作流不会执行。
使用同一数据库前必须停止原服务。新版本不再提供 Hagency CLI 命令、相关
capability、关联入口和 relay/fleet API。

Hagency 业务应用应由独立项目（如
[hagency-server](https://github.com/chrislearn/hagency-server)）维护。
本改动不部署该项目、不验证替代版本、不迁移业务数据，也不自动重定向客户端。
切换既有消费者前，需要另行审阅和验收替代实现。
Matrix 管理员权限本身不授权外部资源分配。
