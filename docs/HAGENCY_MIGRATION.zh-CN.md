# Hagency 应用归属与迁移

[English](HAGENCY_MIGRATION.md)

Palpo 负责 Matrix 客户端/联邦协议、房间、事件、App Service 及 homeserver 管理 API。
Hagency 专用的 fleet 接入、项目申请、资源/coordinator 授权、Agent 预算、Inbox 和
运行端命令投递归属 [hagency-server](https://github.com/chrislearn/hagency-server)。
本次移除独立 Node `web-admin` 及其 CI；通用 App Service 注册、暂停/撤销、URL
compare-and-set、身份 retirement 和 Matrix 管理 API 继续留在 Palpo。

草稿 [#508](https://github.com/palpo-im/palpo/pull/508) 提出的
`palpo-hagency-contract`、`palpo-operations` 尚未合并到 Palpo main。
其中已实现的协议与工作流已在 hagency-server 中改造为 `hagency-contract`、
`hagency-operations`。运行端消费者需先切换来源，再停用草稿中的 Palpo 依赖。
后续合并也不应把这两个业务 crate 加入 Palpo workspace。

## 既有部署

删除源码不会删除运行中的服务、SQLite 数据库、Matrix 账号、App Service 凭据、
房间或待投递命令。既有镜像仍可按原 revision 使用。

1. 准备替代服务时继续锁定可用的 `web-admin` 镜像；备份私有 SQLite 数据库及
   独立的 Palpo/Pasion 数据库。
2. 选择包含协议/Operations 迁移并已审阅的 hagency-server revision。清理 PR
   在该 revision 发布、消费者检查完成前保持草稿。
3. 分别配置 Hagency、Palpo、Pasion 数据库。业务状态属于 Hagency PostgreSQL，
   Palpo 数据库不承接旧应用的 SQLite 导入。
4. 逐项审阅既有 fleet ID、凭据、待处理申请和租约投递的状态迁移。Rust 实现
   没有 Node SQLite 自动转换器，也不会自动将旧 Inbox 变成 coordinator 授权。
   不应靠盲目重建重复 fleet 完成切换、丢弃待处理操作，或同时运行两套业务 writer。
5. 对选定客户端/运行端版本验证 Pasion 登录、Padmin 管理、fleet 投递和新 Inbox，
   再切换流量；保留备份和锁定版本的原部署用于回滚。

新原生接口为 `/_hagency/miniapp/v1/`；Hagency host 在客户端迁移阶段保留
`/_palpo/miniapp/v1/` 别名。这些是应用接口，独立 Palpo 不挂载它们。
Pasion 继续负责身份/OAuth；hagency-rs 负责真实额度预留、Agent 执行与用量回执。
Matrix 服务器管理员身份不等于 Hagency 资源 delegation。

在线服务器关联/delegation 签发、真实 hagency-rs 创建/聊天与 Codex/Claude 配额
执行仍需独立跨项目验收。上游草稿及迁入的 fixture 测试不代表这些流程已可投产。
当前实现及发布边界见选定 hagency-server revision 中的中英文 Operations 说明。
