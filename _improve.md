# Palpo 最新代码审计与改进记录

基线：`main` 最新提交 `59f3d7e9d0`，本轮分支 `chris/codebase-improvements`。

本轮审计聚焦安全边界、未完成实现、未接入功能、死代码/假成功路径，以及配置示例中容易被复制到生产环境的风险。

## 本轮已完成

- [x] 修复 URL preview allowlist/denylist 判定顺序。`domain_explicit_denylist` 现在会优先于 `*` allowlist 生效，避免管理员配置了全量 allowlist 后显式 denylist 被绕过。
- [x] 修复 URL preview `domain_contains_allowlist` 匹配方向错误。配置 `google.com` 现在会按文档匹配包含该片段的 host，而不是反向检查“配置项是否包含 host”。
- [x] 收紧 URL preview contains 匹配。空字符串不再被当作“匹配所有 URL/host”的 contains 规则。
- [x] 为 URL preview allowlist/denylist 行为补 3 个单元测试，覆盖 contains 匹配方向、denylist 优先级和空 contains 项。
- [x] 修复本地 `/keys/query` 指定多个 device id 时只返回最后一个设备的问题。设备结果容器现在在循环外创建，不再被每个 device 覆盖。
- [x] 替换 `palpo-required.toml`、`palpo-required.kdl` 和 Discord bridge 配置注释中的 `postgres:root` 示例密码，保持与其它示例一致的强占位符。

## 仍需专项处理

- [ ] `POST /_matrix/client/*/account/deactivate` 注释声明会离开房间、清设备、清 to-device 等，但当前路由只调用 `data::user::deactivate`，没有接入已有的 `full_user_deactivate` 完整流程。
- [ ] E2EE cross-signing 仍有未完成校验：`add_cross_signing_key_updates` 标注 `TODO: Check signatures`，`keys/signatures/upload` 也只是持久化签名并固定返回空 `failures`。
- [ ] MSC3391 account data delete 路由当前返回成功但没有删除数据，属于未实现功能返回假成功。
- [ ] 多处 Matrix 协议缺口仍以明确错误返回，例如 dehydrated devices、third-party invite exchange、3PID bind 等，需要按协议功能专项补齐。
- [ ] 本地工作区存在被 `.gitignore` 排除的 `examples/with-*/data` / `space` 运行数据，扫描到 token-like 值。它们不在 Git 跟踪内，但建议开发环境定期清理，避免误打包或手工复制。

## 验证

- [x] `cargo check -p palpo --bin palpo -j 1`
- [x] `cargo test -p palpo media::preview::tests -j 1`
- [ ] `cargo check --workspace --all-targets -j 4` 本轮曾运行，但 4 分钟超时，未得到完整 workspace/all-targets 结果。
