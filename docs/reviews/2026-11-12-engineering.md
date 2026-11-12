# 工程质量与运维评审

评审日期：2026-11-12。源码基线：`61b2ad7f02e216e6292116abfa3bcdd8e85cae25`。修复状态：ENG01–ENG05 已在本分支实现。批量撤销复用持久通知事务；删房实际退房与移除别名；未实现媒体库存明确拒绝；发布调用同 SHA 门禁并待所有平台产物齐备后公开；readiness 文档链接现有 diagnostics。新增隔离 PostgreSQL/HTTP 故障注入回归，运行结果由 PR 记录。下文保留原审查证据，行号对应审查基线。

本分册聚焦管理操作的真实结果、错误语义、交付链和运维资料。下述问题为静态调用链确认，尚未对运行中的服务做故障注入。

## ENG01 批量设备撤销和 MAS 对账吞掉删除失败

优先级 P1，置信度高。

位置：`crates/server/src/routing/admin/user.rs:158–175`，`crates/server/src/routing/admin/mas.rs:262–284`；底层 `crates/server/src/user.rs:229–233`，`crates/data/src/user/device.rs` 的 `remove_device` 和 `remove_device_on_connection`。

管理员批量删除设备，以及 MAS 将设备集同步为目标列表时，循环使用 `let _ = ...remove_device(...).await`，最后始终返回成功。底层删除包含数据库事务，失败会回滚设备、access token、refresh token 等删除；也可能在本地删除提交后，设备列表更新或联邦通知阶段失败。

因此，“设备仍然存在且 token 有效”与“设备已删但通知失败”都会被接口隐藏。调用者无法可靠判断撤销是否完成。单设备删除接口会正常传播错误，同一模块的行为不一致。触发条件包括事务内语句失败、数据库短暂故障或通知持久化失败；本报告没有声称普通成功调用也会留下设备。

建议：复用已有批量设备撤销的事务和持久通知模式；至少传播失败或返回逐设备结果，区分撤销失败与通知待重试。MAS 对账不得在未达到目标设备集时返回完全成功。对不存在设备的幂等行为应单独定义，不能通过吞掉全部错误实现。

验收：在删除事务中注入错误，验证调用非成功且原 token 状态可解释；在提交后通知阶段注入错误，验证持久重试最终完成；多设备请求包含成功、缺失和失败项时，响应准确反映实际结果。

## ENG02 删房接口报告已踢出成员但没有执行退房

优先级 P1，置信度高。

位置：`crates/server/src/routing/admin/room.rs:516–558`，`crates/server/src/room.rs:143–150`、`:875`。

`delete_room` 把当前所有本地成员直接收集为 `kicked_users`，随后只执行可选 ban、设置 `rooms.disabled` 和可选历史清理。它没有逐成员生成 leave 事件，没有实际退房，也没有删除所返回的本地 alias；响应仍将失败列表设为空。`disable_room` 仅更新布尔列，`ban_room` 仅维护 banned_rooms，二者都不能替代成员状态变更。

另外，`purge=true` 分支吞掉历史清理错误。该接口默认 purge 为 true，容易使管理工具把一次部分操作记录成完成。源码注释又声称 purge 没有删除数据库事件，实际已调用有删除实现的函数，说明接口契约和注释都需要重新核对。

建议：定义删房流程和完成状态，实际执行并记录成员退房、alias 处理和历史清理；规模较大时改为可查询状态的持久作业。只有已完成的成员才进入 `kicked_users`。若当前仅提供禁用能力，应返回对应结果而非伪造踢人成功。

验收：创建含本地成员和 alias 的房间，调用删房后核对 membership、成员 sync、alias 解析及历史可读性；强制 purge 失败时响应不得宣称全部完成。历史清理本身的容量与数据结构风险另见并发性能分册。

## ENG03 房间媒体列表用成功空数组掩盖未实现能力

优先级 P2，置信度高。

位置：`crates/server/src/routing/admin/media.rs:26`、`:184–197`。

已注册的房间媒体列表路由忽略 room_id，固定返回 `local: []`、`remote: []`。存在真实媒体时，清理或排障工具仍会得到“没有媒体”的成功结果。这是管理扩展的结果真实性问题，不据此认定 Matrix 核心协议不符合。

建议：实现有界分页的媒体引用查询；此前明确返回不支持，或从公开管理能力中撤下该路由。不要用空结果模拟实现。

验收：包含本地及远端 MXC 的房间可查到引用；不存在房间、权限不足、暂不支持与真实空结果有可区分的响应。

## ENG04 发布流程缺少仓库内可见的同 SHA 质量门禁

优先级 P2；代码事实置信度高，部署暴露取决于仓库外部保护规则。

位置：`.github/workflows/release-docker-images.yml:3–8,95–117,138–139`，`.github/workflows/release-binaries.yml:29–49`。

镜像发布由 main/next 推送和版本 tag 直接触发，merge 只依赖镜像 build；未在该流程内等待同一 SHA 的 quality-control / Complement 成功。二进制发布先创建 `draft: false` 的 release，后构建上传各平台包，因此部分构建失败时可能留下公开但不完整的 release。

这是交付防护缺口，不是已经发生错误发布的证据；GitHub branch rules、environment protection 和 tag protection 属于本次未读取的远端配置，不能断言不存在。

建议：将发布前验证做为同 SHA 的必要依赖或受控提升步骤；先生成草稿，全部产物及校验清单齐备后再公开。保留手动发布时也需要遵守的可审计例外机制。

验收：故意使一个必要测试或一个平台构建失败，确认版本不会被提升为可用完整发布；重复运行不会混入另一 SHA 的产物。

## ENG05 Readiness 文档与新增 diagnostics 能力未同步

优先级 P3，置信度高。

位置：`docs/operations-readiness.md:37–43`，`docs/development/outbound-diagnostics.md:3–35`，`crates/server/src/observability.rs:30–34,109,387`。

Readiness 文档仍声称 outgoing request 没有入队时间、相关 queue age / heartbeat / histogram 未实现；新增 diagnostics 已提供可空入队时间、队列年龄、worker heartbeat 和延迟桶。应保留“readiness 本身不做队列聚合”的正确边界，同时说明这些数据已在 `/diagnostics` 提供，旧行年龄仍未知，不能笼统说不存在。

验收：两个文档分别明确 liveness、readiness、diagnostics 的职责，链接单一指标说明；保留采样预算、旧行未知年龄及进程本地计数限制。

## 工程改进建议

以下是建设项，不计入已确认缺陷数量。

- 将设备撤销、房间销毁、媒体清理做成具有明确提交点和可观察结果的操作，统一“对象不存在”“部分完成”“存储失败”“等待通知”的错误契约。
- 对管理路由建立能力清单与行为测试，尤其筛查固定空数组、固定成功结果和过宽的 `.ok()` / `let _`。不要把搜索命中数量当成缺陷数量。
- 增加最小特性、默认特性、重要 MSC 组合测试；all-features 通过不能证明每个特性单独启用也正确。CI 目前已有部分 default/all-features 隔离测试，可在其基础上扩展。
- 构建关键 Actions 使用可审计 commit pin，镜像基础版本与 toolchain 记录到产物元数据；`build/docker/Dockerfile.palpo:13` 构建采用 `--locked`，明确锁文件漂移应失败的预期。当前没有证据证明已发生供应链入侵或依赖漂移。
- 根据现有 PostgreSQL 数据、签名私钥、媒体对象及配置的联合依赖编写完整恢复演练；不要把局部 key-history 导出恢复测试当成整站灾备演练。

## 本轮实际验证

主报告记录本机命令、退出状态与测试数。本分册的故障触发场景是建议加入的回归验收，未伪装成已经执行的集成测试。评审不修改业务源码、不发布产物。
