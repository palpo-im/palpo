# Palpo 多维代码评审与改进报告

评审日期：2026-11-09（Asia/Shanghai）
代码基线：`23535fc449b9919f9a34a54608889bced930c156`，workspace 版本 `0.5.0`。评审开始时工作区干净。
执行方式：主评审 + 三个专项 agent（安全、Matrix 协议、并发/性能/可靠性），主评审交叉检查关键调用链并补充测试工程、供应链与运维维度。首次评审仅新增报告；后续按用户要求分批并行修复 R01–R07、R08–R10。

## 修复状态（2026-11-09，R01–R07 已修复并验证）

以下状态对应当前工作区；后文保留原始评审证据与剩余工作，原始行号对应评审基线。

| 项目 | 已实现修复 | 回归覆盖 |
|---|---|---|
| R01 | 三种 state GET 共享成员快照边界，传播 DB/frame 错误；用成员 before-state 证明加入经历，后续 ban/unban 不推进离房边界 | 真实 HTTP：leave/kick/ban、topic/member/custom state、world-readable、rejoin、缺失 frame；预先封禁、解封及拒绝邀请的非成员拒绝访问 |
| R02 | 持久 reservation、owner/server/expiry 校验，跨实例 fence+行锁下发布；metadata/uploaded 原子提交 | HTTP 跨用户/未预留/异 server/过期拒绝，并发唯一成功；真实 FS/PG 故障与恢复 |
| R03 | 统一下载 disposition/nosniff，覆盖无 filename、缩略图回退；客户端 S3 改代理 | HTTP HTML/显式 filename/回退 attachment、PNG inline；未连接真实 S3 |
| R04 | v5+ 按事件时间筛钥，fetch-time 七天上限，过期缓存刷新；旧钥不继承新钥有效期，本地退役钥同样校验 | 实际签名 verifier：v4/v5、有效期边界、多签名、退役/本地钥；HTTPS notary 刷新、参数、缓存命中、503拒绝 |
| R05 | 传输层单次获取 semaphore，移除事务发送外层许可 | 容量 1、8 个并发请求必须通过限流后到达 URL 安全拒绝；permit 全归还 |
| R06 | 标准 push 响应解码、状态检查、失败传播；精确匹配原注册清理 rejected，避免删更新后的注册 | 网关协议测试、注册竞争；真实 HTTP→消息→持久队列→503→200 自动重试 |
| R07 | ACK/claim/后续读取失败留在循环恢复，supervisor 保留 receiver；取消清理 worker 状态 | PG DELETE/UPDATE trigger 注入、健康目的地继续、恢复自动清队列；启动 SELECT 失败恢复 |

CI 已加入默认/all-features 两种模式下的十项独立回归，每项独立进程和空库，并校验测试名确实注册。集成测试故意标为 ignored，通过显式 CI 执行；新增独占测试使用 `review_` 命名，避免混入旧的 `database_` 同进程测试批次。

升级注意：新增 `2026-11-09-000000_media_upload_reservations`，须停旧 writer 协调升级；已上传媒体可读，升级前未使用且无 owner 记录的 MXC 必须重新创建。客户端 S3 下载改代理会增加 homeserver 带宽。详见 [UPGRADING](../UPGRADING.md)。

本轮验证环境：Windows x86_64、Rust 1.99.1、项目正常 target 目录内的独立 PostgreSQL 18（loopback 25439），未使用 Docker。默认特性构建通过，普通 server lib 测试 **314 passed / 0 failed / 83 ignored**；上述十项独占回归在独立进程和空库中另行执行，**10/10 通过**，其中推送与 worker 恢复均实际等待约 30 秒退避。默认回归二进制 SHA-256：`1903D0E57DCD2540B02C42549E2A57C588F5506B6DE13CAB5CFB9B24D05B4655`。

all-features 构建同样通过，普通 server lib 测试 **359 passed / 0 failed / 108 ignored**；上述十项独占回归再次 **10/10 通过**。all-features 回归二进制 SHA-256：`EEE1B561818470E6BD5B924AAE659534212C4AAA4E6AA7C8CEB1237361789964`。两种模式中的 ignored 数量是普通单测批次的统计；这十项独占回归在该批次之外显式执行，没有将它们的 skip 当作通过。

严格静态检查通过：`cargo +1.98.1 clippy --workspace --all-targets -- -D warnings`（与 CI 固定版本一致）、`cargo +nightly fmt --all -- --check`、`git diff --check`。Python 结果判定器 **25/25 通过**，新增 CI Bash 步骤通过语法检查。

最后的 Clippy 整理仅将 R04 的三处等价嵌套 if 合并为 let-chain；最终源码重新构建，并在两种模式下额外复验 R04 的 **7 项普通测试与 3 项独占 PostgreSQL/HTTPS-notary 回归**。所有结果通过。

本机证据保存在正常 target 目录：`target/review-results/{default,all,finaldefault,finalall}/`、`target/review-{default,all}-unit-tests.log`、`target/review-clippy.log`，未纳入版本控制。独占用例的完整名称和可复现 Cargo 命令见 `.github/workflows/quality-control.yml` 的 `Security, protocol and resource review regressions (R01-R10)` 步骤（后续扩展为十三项）。普通批次可用 `cargo test --locked -p palpo --lib` 和追加 `--all-features` 运行；独占用例必须各用一个空库、独立进程及 `--ignored --exact --test-threads=1`。

验证范围：真实 PostgreSQL、真实本地文件存储、真实 loopback HTTP/HTTPS peer；数据库故障通过事务 trigger/表暂不可用注入。初次本地验证未运行完整 Complement/Sytest、真实 S3、跨主机网络分区、进程强杀/崩溃恢复或压测，不将这些未覆盖场景计为通过。R08–R10 的后续状态见下节；O01–O02 保持未修复。

### PR 评审补充

[PR #636](https://github.com/palpo-im/palpo/pull/636) 的首轮 Codex 评审指出，预先封禁一个从未加入过房间的用户会创建 ban 行，因此不能将 ban/leave 行本身作为曾加入的证明。补充修复沿服务器保存的成员 before-state 查找真实加入经历，并固定到结束该次加入的首次离房事件；后续封禁、解封、邀请与拒绝不推进旧边界。缺少加入证明的非成员不能读取非 world-readable 房间状态。

同一个双模式 HTTP 回归已扩展，覆盖预先封禁、解封、邀请后拒绝，以及离房/被踢后追加封禁；所有三种 state GET 均核对。上文测试计数与二进制摘要保留初次完整本地验证证据；PR 中新增断言的最新 Linux CI 与再次评审结果见该 PR 的检查和评审记录，合并以最新提交的检查通过及评审无未解决问题为条件。

## R08–R10 修复状态（2026-11-10）

本批基于已合并 R01–R07 的 `9143e58cd91f5b089eec87cdfb049298e65dcb03`。三个 agent 分别修复 R08、R09、R10，主 agent 交叉检查、补真实混合联邦回归并统一验证；R09/R10 另作互相只读评审。以下描述实现与验收范围，不替代后面的运行证据。

| 项目 | 已实现修复 | 回归覆盖 |
|---|---|---|
| R08 | `/keys/changes`、v3/v5 sync 共用不可变 publication 快照；`left` 比较共享加密用户集合，保留普通房间新增共享及密钥更新通知；事务 journal 保存历史变更窗口 | 真实 HTTP keys/v3，与 v5 service 响应对照；自己/对方 leave、ban、最后/多个共同房间、明文共享、多设备、rejoin、历史 to、相邻窗口、token 后缀/非法 token、真实签名 outlier 延迟发布 |
| R09 | 类型含 null/custom 过滤，远端保留网络参数；本地命名第三方网络空集，All 包含现有 Matrix；联邦 GET/POST 参数与认证一致；过滤后计数、稳定平局排序及双向分页 | 真实 HTTP client/签名 federation GET/POST、网络冲突、计数及正反分页/改变 limit；HTTPS peer 验证远端透传；混合 Palpo↔Synapse 增加类型过滤和正向分页 |
| R10 | 16 个预览 cache-miss 请求预算（含同 URL 等待者），显式 lease 计数及 RAII 回收；每个限速器最多 65,536 桶，只回收完全恢复的桶，满额拒绝新键 | 5 项普通资源回归：并发最后释放、同键互斥、取消、10,000 唯一键、容量/债务/索引/时钟；独占真实 PG/HTTP：128 失败 URL、12 并发同 URL 仅一个 producer、网络中途取消 |

R08 新增 `2026-11-10-000000_device_key_change_history`。trigger 覆盖旧 writer，回填仅保存迁移时尚存 marker；迁移前已被覆盖的密钥历史无法重建，客户端应刷新跟踪设备列表。读取端全部升级前响应语义可能不同；回滚数据库前须停止新读取端。详见 [UPGRADING](../UPGRADING.md)。

CI 的十三项独占回归在默认/all-features 各自空库与独立进程中执行，并验证精确测试名已注册；五项普通资源回归也在两种模式显式执行。混合联邦新增子用例位于既有必过 `TestMixedPublicRooms` 下，不依赖全套比例放行。

本批运行证据正在统一验证，合并仍以最终提交复评无未解决问题、全部 CI 通过为条件。

边界：v5 通过 service 函数响应比较，未新增 v5 HTTP 路由端到端用例；目录 offset 分页保证静态目录，未承诺并发目录变更的快照；第三方网络发布仍未实现；预览预算从首次缓存 DB miss 后生效，失败请求无负缓存；限速桶惰性回收但容量始终有界。未进行 RSS/QPS 压测、进程强杀或跨主机网络分区演练。O01/O02 及报告中的其余专项工作不计入本批已修复。

## 原始评审结论与阅读方式（修复前）

项目已有较完整的 homeserver 骨架、真实 PostgreSQL 回归测试、混合 Synapse 联邦测试及多层出站网络防护。但本轮确认的权限边界和出站失败路径问题，足以影响公开部署的隐私、媒体完整性及消息交付。建议先修复 R01–R07，再以可重现回归证明其行为，随后推进协议一致性和规模优化。暂不建议以现有 CI 通过或历史测试数字作为生产就绪证明。

共收敛 **10 项代码问题：7 项 P1、3 项 P2**；另列 2 项可观测性/供应链门禁缺口。P1 表示应优先修复的隐私、完整性或核心可用性问题；P2 表示协议功能、资源边界或工程保障问题。没有足够依据认定 P0。所有代码问题为静态调用链确认，尚未做运行时漏洞利用或故障注入；严重性不等于已观测到生产事故。

不打主观“完成度百分比”或综合分：类型定义、路由存在、单机成功、真实异构互通和故障恢复是不同证据等级，不能互相替代。

## 原始评审已执行验证与边界

- 读取 workspace、关键路由/服务/数据层、CI、测试清单和升级文档；由专项 agent 核对相关 Matrix 官方规范。
- `cargo metadata --no-deps --format-version 1` 成功；返回正常产物目录 `D:\Works\palpo-im\palpo\target`。未重定向 Cargo home、target、TEMP/TMP，未使用全局替代缓存。
- `python -m unittest discover -s tests -p '*_test.py'`：**25 项通过**。这是测试结果判定器的单元测试，不是 homeserver 协议、联邦或性能测试。
- 仓库中的 `tests/results/test_all.result.jsonl` 有 672 条 pass、14 条 skip；该文件最后提交于 **2026-04-07**，且缺少原始 Package、run、环境和二进制信息，不能代表本次提交的通过率。
- 未执行 Rust 编译/全量单测、真实 PostgreSQL 回归、Complement/Sytest、渗透或压测。未启动 Docker；因此没有触及本机 Docker 数据盘，也没有本轮实时互通结论。
- 这是风险导向抽样评审，不是逐行完整审计；state resolution 全部房间版本、全部 MSC、所有迁移和所有部署组合仍需后续专项验证。

## 正交维度总览

| 维度 | 已见基础 | 当前主要风险/下一步证据 |
|---|---|---|
| 认证授权与隐私 | 集中 auth hoop、appservice 身份绑定、OAuth scope 回归 | R01 离房 state 边界；R02 媒体 owner 绑定 |
| 密钥与联邦信任 | 签名/事件授权实现、key 缓存、notary 配置 | R04 PDU key 有效期；补轮换、过期、恶意签名 peer |
| 媒体与浏览器隔离 | 内容大小、thumbnail 解码资源限制、disposition helper | R02 reservation；R03 无 filename 的响应策略 |
| 协议完成度与响应一致性 | 广泛 CS/S2S 路由、sync v3/v5、设备/密钥实现 | R08 keys changes、R09 publicRooms；建立版本化功能矩阵 |
| 并发与活性 | Tokio、持久发送队列、有界 wakeup、DB coordination pool | R05 重入 semaphore；证明低容量与饱和时可前进 |
| 持久性与故障恢复 | 发送先入库、补扫、启动恢复、部分幂等回归 | R06 推送丢失、R07 worker 退出；补 DB/网络故障注入 |
| 效率与扩展性 | 批处理上限、独立 query/coordination 预算 | 出站 N+1、目录全量构建；先采 SQL 数与尾延迟 |
| 内存与资源边界 | 网络/媒体限制、有界通知 channel | R10 锁表及限速桶长期增长；补容量与取消测试 |
| 可观测性与运维 | tracing、连接池状态、升级顺序说明 | O01 health 恒成功；缺少 worker/队列 readiness 信号 |
| 测试与发布保障 | 严格 required-case gate、双向 Synapse 混合测试 | 全套按比例放行、历史结果不可复用、故障场景缺口 |
| 供应链 | Cargo.lock、deny 来源/许可证策略 | O02 advisories 全部降为 warning；lock-only 联邦 CI 漏触发 |
| 可维护性与架构 | core/data/server 分层、过程宏集中 | 全局状态、多职责大模块、吞错模式；按边界渐进拆分 |

## 代码问题及验收要求

### R01 · P1 · 批量 state 接口向已离房用户返回后续状态

**证据：** `crates/server/src/routing/client/room/state.rs:37` 计算 `_until_sn`，`:54` 却调用 `room::get_frame_id(&room_id, None)`。同文件单项接口 `:166` 正确传递 `until_sn`，两者行为不一致。

**触发与影响：** 非 world-readable 房间中，A 入房后离开，B 再修改 topic、成员资料或其他状态；A 调用批量 `/rooms/{room_id}/state`，仍有离房记录可通过入口检查，却读取当前 frame，从而看到离房后的状态。这里不宣称 A 能读取全部后续 timeline。

**改进：** 在批量读取中应用离房时的快照边界，并明确快照缺失时拒绝/报错；避免 `unwrap_or_default` 把数据库或状态错误伪装成成功空数组。统一批量、单项及空 state_key 的可见性策略。

**验收：** 双用户真实 HTTP 回归覆盖离房前后 topic/member 变化、kick/ban、重新加入、world-readable；批量与单项读取得到相同边界。规范：[读取房间状态](https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3roomsroomidstate)。

### R02 · P1 · 异步媒体 URI 没有 reservation 与 owner 绑定

**证据：** `routing/client/media.rs:155` 的 create 只返回随机 URI；`:223` 的 PUT 忽略 `_depot` 身份及 `args.server_name`，`:238` 起只判断本地存储键不存在便写入，元数据 `created_by: None`。以下路径均相对 `crates/server/src/`。

**触发与影响：** A create 后先发布待上传 MXC，B 获知其 media ID 后用自己的合法 token 先 PUT；B 的文件占用 URI，A 上传失败。无需猜随机 ID。未 create 的任意 ID 同样可以 PUT；检查存在再写的过程也缺少并发唯一成功保证。

**改进：** 持久化本地 server、owner、expiry、pending/uploaded 状态；在读大 body 前授权；以原子 claim 和不可覆盖对象写入完成状态迁移，记录 uploader，并设计 DB/对象存储失败后的清理与恢复。

**验收：** A create/B PUT→403；未 reservation→404；异 server→拒绝；A 首次成功；重复及并发 PUT 唯一成功，DB 和对象内容一致。规范：[异步上传 PUT](https://spec.matrix.org/latest/client-server-api/#put_matrixmediav3uploadservernamemediaid)。

### R03 · P1 · 无 filename 的本地媒体下载遗漏安全 disposition

**证据：** `crates/server/src/routing/client/media.rs:80` 设置上传 Content-Type，但 `:81` 仅当 filename 存在才加 Content-Disposition。上传允许省略 filename。`routing/media.rs:48` 附近 legacy download 无认证；`config/media.rs:20` 默认启用 legacy，freeze 不关闭下载。源码全局响应路径未见 CSP/sandbox/nosniff 防护。

**触发与影响：** 认证用户上传无 filename 的 HTML，受害者访问 legacy 下载地址，浏览器可在媒体 origin 执行脚本。同源部署 Web 客户端/管理页面时风险扩大；独立媒体 origin 时影响较小。不能由此直接断言任意 Matrix token 可被窃取。S3 presign redirect 的响应策略需要另行验证。

**改进：** 所有下载均调用现有 `make_content_disposition`，不以 filename 存在为条件；HTML/SVG 等危险类型强制 attachment，补 nosniff，并检查缩略图原文件回退与 S3 分支。可结合独立媒体域部署。

**验收：** 有/无 filename、HTML/SVG/PNG、本地/S3/回退路径的 HTTP header 契约测试；危险类型下载不能作为同源活动文档执行。规范：[Serving inline content](https://spec.matrix.org/latest/client-server-api/#serving-inline-content)。

### R04 · P1 · PDU 事件验签丢弃 key 有效期

**证据：** `event/handler.rs:440` 起解析时间为 `_origin_server_ts` 后弃用；`server_key/verify.rs:49` 起调用 `get_event_keys` 再密码学验签。`server_key.rs:88` 的 `verify_keys_for` 与 `:122` 的 `merge_old_keys` 把 old key 转为无 `expired_ts` 的 VerifyKey；`:200` 缓存命中立即返回，无事件时间检查。`global.rs:350` 附近虽有过滤器，实际事件路径未调用。以上相对 `crates/server/src/`。

**触发与影响：** room v5+ 事件时间晚于 key 有效期或 old key expired_ts，当前链仍可能使用该公钥验证新事件。失效/轮换边界失效；持有退役私钥者仍可能通过签名层。后续房间事件授权仍存在，本条不代表任意权限绕过，也不套用于 HTTP 请求签名策略。

**改进：** 保留时间元数据，按事件时间和 room version 选钥；无有效钥时刷新而非无限信任缓存；按规范处理有效期上限。不要简单把旧过滤器接上就结束：历史事件、key rotation、边界时间和未来七天上限必须分别核对。

**验收：** v4/v5+ 对照、有效期前后、old key 前后、缓存刷新、notary 失败、远期有效期；使用可编程签名 peer。规范：[Room v5 signing key validity](https://spec.matrix.org/latest/rooms/v5/#signing-key-validity-period)、[Server key API](https://spec.matrix.org/latest/server-server-api/#get_matrixkeyv2server)。

### R05 · P1 · 联邦发送重复获取同一 semaphore 导致死锁

**证据：** `crates/server/src/sending.rs:786` 获取全局 max_request 许可，持有期间 `:809` 调用 `send_federation_request`，后者 `:846` 再获取同一 semaphore。网络 timeout 在内层取得许可后才开始。

**触发与影响：** `max_concurrent_requests=1` 时单次联邦发送即可等待自己的许可；更高容量下，多个目的地在外层许可与内层许可之间因 discovery await 暂停，也可耗尽全部许可。超时无法解除许可等待，联邦投递停止。

**改进：** 明确唯一的许可所有者，通常由传输层统一获取一次；定义 DNS、连接、body 消费的预算及 permit 生命周期。等待许可也应具备取消/可观测性，但不能用超时掩盖重复获取。

**验收：** 容量 1、目的地数等于容量、慢 discovery、混合 push/appservice/federation；有界时间内完成或明确失败，失败/取消后 permit 全部归还。

### R06 · P1 · 推送错误被当作成功，持久请求被删除

**证据：** `sending.rs:738` 将 `send_push_notice` 的 Result 存入 `_response` 后弃用，`:751` 无条件 Ok；`sending/guard.rs:166` 在成功路径删除 active 请求。另 `user/pusher.rs:224`、`:240` 使用 `send_by_client::<()>`，而 `crates/core/src/sending.rs:113` 起确实 JSON 反序列化；合法的 `{"rejected":[]}` 无法解析为 unit，非空 rejected 也未处理。仓库已有 `crates/core/src/push/push_gateway.rs:39` 的响应类型。

**触发与影响：** 网关断连、超时、错误状态或解码失败均可被吞掉，通知不重试。合法网关可能已经发送通知，因此不能把解码错误描述成“所有通知都未送达”；确定的是本地无法正确区分成功、失败和失效 pushkey。

**改进：** 传播失败、保留持久队列并退避重试；为响应类型补充 Deserialize，解析标准响应并移除 rejected pushkey；检查 HTTP 状态。**必须同时修正响应解码和错误传播**，否则只加 `?` 会让正常响应持续重试。批内部分成功要定义至少一次与去重语义。

**验收：** 网关先 503/断连再恢复；正常 rejected 空列表；失效 key；批内部分失败；队列保留、最终投递及失效 pusher 清理。规范：[Push Gateway notify](https://spec.matrix.org/latest/push-gateway-api/#post_matrixpushv1notify)。

### R07 · P1 · 一次数据库错误可永久终止统一出站 worker

**证据：** `crates/server/src/sending/guard.rs:166` 删除 active 行、`:177` claim 下一批都用 `?` 退出 `process()`；`:51` 启动任务只打印错误，未监督重启或报告失活。

**触发与影响：** 网络成功后的 DB 连接池超时/瞬时错误即可令统一 worker 结束，数据库恢复后仍不恢复；联邦、appservice、push 队列可能持续积压，直到进程重启。不是每个数据库故障都会丢已持久化数据，但会失去持续处理能力。

**改进：** 可恢复 DB 错误在循环内退避并恢复状态；增加任务监督、退出信号和 readiness/告警；重启从持久队列恢复，避免依赖内存中尚未确认的窗口。

**验收：** 在发送后/删除前及 claim 时注入 DB 故障，恢复数据库后无需重启自动继续；验证队列不丢、幂等窗口不错误推进、其他目的地仍可恢复。

### R08 · P2 · `/keys/changes` 永远不返回 left

**证据：** `crates/server/src/routing/client/key.rs:123` 起按当前 joined_rooms 查询 changed，`:152` 固定 `left: Vec::new()`。

**影响：** 最后一个共同加密房间解除共享后，客户端无法从该接口获知应停止跟踪的用户，设备跟踪状态和同步接口可能不一致。不能据此推断服务器能解密 E2EE 内容。

**改进与验收：** 按 from/to 的共享关系变化计算 left，覆盖自己/对方离房、最后共同房间消失、仍有另一共同房间、离开再加入、多设备及跨页；与 sync device_lists 对齐。规范：[keys changes](https://spec.matrix.org/latest/client-server-api/#get_matrixclientv3keyschanges)。

### R09 · P2 · publicRooms 过滤与能力声明不一致

**证据：** `crates/server/src/directory.rs:51` 起本地实现只应用 generic_search_term，不使用 `room_types`；远端转发 `:31` 固定 RoomNetwork::Matrix，本地 `_network` 也被忽略；`routing/client.rs:274` 却声明 `org.matrix.msc3827=true`。

**影响：** 请求仅列 Spaces 或普通房间时会混入其他类型；网络参数未按请求保留，客户端无法依赖声明做功能选择。这也是现有 `tests/complement/MIXED_FEDERATION.md` 明示的实现缺口。

**改进与验收：** 实现类型（包括 null/无 type）和网络过滤，或停止声明相应能力；补本地/远端、GET/POST、过滤后计数及双向分页完整性测试。规范：[publicRooms POST](https://spec.matrix.org/latest/client-server-api/#post_matrixclientv3publicrooms)。

### R10 · P2 · URL preview 锁表永久增长

**证据：** `crates/server/src/media/preview.rs:17` 全局 URL→Arc<Mutex> HashMap 只插入；`:269` 缓存 miss 后分配，即使网络失败也不删除。入口 `routing/client/media.rs:289` 先检查 URL allow 配置，所以需启用预览且 URL 被允许。

**影响：** 允许域名上附加不同 query 就能持续制造常驻键，长期或恶意负载增加内存。代码能证明无界增长，不能给出未经测量的 OOM 时间或 QPS。

**改进与验收：** 使用可回收的 keyed mutex/有界表及整体并发预算；避免简单 remove 使等待者与新请求分裂成两把锁。验证大量失败 URL、同 URL 多等待者、取消/超时，闲置后容量有界且同 URL 不重复抓取。`hoops.rs:62` 起限速 bucket map 也无回收，纳入同一资源生命周期治理。

## 运维与工程保障缺口

### O01 · P2 · 当前 health 只能证明 HTTP 活着

`crates/server/src/routing.rs:138` 恒回 200/ok，不能反映 DB 或 R07 中出站 worker 已停止。它作为 liveness 可以成立，但不足以承担 readiness。建议独立 `/ready` 检查启动/关键 worker 状态及有界 DB 探测；暴露 queue oldest-age、pending/active/retry 数、worker 退出/重启、pool wait、permit wait、出站错误分布。DB 断连及 worker 退出时 readiness 和告警必须变化；liveness 避免因下游短暂异常造成重启风暴。嵌入式模式应提供状态 API 由宿主接入。

### O02 · P2 · advisories 不阻断 CI

`.github/workflows/quality-control.yml:47` 把 vulnerability、notice、unmaintained、unsound 全部降为 warning；因此 cargo-deny job 绿色不表示没有已知漏洞。本轮没有运行在线 advisory 扫描，不断言当前依赖存在某个 CVE。建议漏洞默认阻断，仅以带 owner/原因/到期日的精确例外豁免；notice/unmaintained 与可利用漏洞分别治理。保留已有来源和许可证约束。

## 完成度与测试证据应如何解释

| 范围 | 当前可见证据 | 还需要的证明 |
|---|---|---|
| 基础 CS API / sync | 广泛实现及大量回归用例 | 在本次 SHA、固定 binary 上执行 default/all-features；验证离房边界与错误响应 |
| 真实异构联邦 | 固定 Complement revision，32 个 manifest 项双方向门禁；文档明确除外场景 | 当前 SHA 运行结果、二进制摘要；网络分区、重启、坏签名、过期钥 |
| 事件授权/state resolution | core 单元测试与服务实现 | 分歧 DAG、多房间版本、rejected/soft-failed、缺失前驱恢复和差分测试；本轮未全面证明 |
| 设备/E2EE 支持 | OTK/fallback/cross-signing/recovery 与原子性回归 | R08；真实 SDK 的加密/解密、验证与恢复；opaque encrypted PDU 转发不等于端到端流程成功 |
| 媒体 | 认证/legacy、本地/S3/remote/thumbnail 路径 | R02/R03、流中断、并发上传、对象存储失败与 header 一致性 |
| 目录/Spaces/成员 | 真实互通清单和专项测试 | R09、过滤/分页、拓扑和权限变化组合 |
| 第三方身份邀请 | `routing/federation/event.rs:209` 和 `threepid.rs:11` 明确返回 M_UNRECOGNIZED（相对 server/src） | 明确未实现功能，单列完成度缺口；`3pid/onbind` 当前挂载 PUT，未来实现需对照规范 POST |
| 可选 MSC | 类型、feature gate、部分专项回归 | 为每个声明列出 spec revision、编译组合、真实 HTTP 用例和支持限制 |

CI 已有值得保留的严格设计：required-case gate 要求显式 PASS、拒绝缺失/skip/重复终态/截断，混合联邦检查实际镜像身份；不能简单把 `continue-on-error` 当作“所有失败都被忽略”。但默认全套 Complement 仍采用 98% 阈值，零失败模式只在手工参数启用；不在 required 清单中的新回归可能被其他通过用例稀释。建议改为具名、限期的已知失败 baseline，阻断新增失败/skip；核心安全与投递回归全部进入 required 清单。

此外，`.github/workflows/complement.yml:11`、`:22` 的 paths 没有 Cargo.lock，纯 lock 更新不会触发联邦 job；应补充。Synapse fixture 从默认分支构建（同文件 `:260`），有兼容性预警价值但不易复现；建议增加固定 revision 的必过 lane，保留 latest lane 作为前瞻监控。每次产物记录 Palpo SHA/binary SHA-256、Complement/Synapse revision、features、配置摘要、原始 Package+run+terminal 日志和退出码。

数据库测试使用独立空库防止破坏已有数据，这是正确约束。需维护忽略测试清单与 CI 映射；例如 `lib.rs:498` 的 embedding PostgreSQL 用例使用独立变量，本轮未在 workflow 找到对应执行，应避免把普通 `cargo test --all-features` 等同于已执行全部集成测试。

## 效率与可维护性后续工作

以下是代码层面可见的优化机会，**尚无压测证据证明它们是当前最大瓶颈**：

1. `sending.rs:1007` 的 queue_requests 逐条获取连接/INSERT；广播目的地多时往返线性增长，中途失败留下部分入队。评估批量 INSERT，并先定义业务原子性。
2. `sending/guard.rs:570` 起设备列表和 `:623` 起 receipt 按房间/用户串行查询，且位于统一发送调度循环；记录每批 SQL 次数与调度停顿，再改批量 join/增量索引。
3. `sending.rs:759` 起逐 PDU 读取与转换；批内重复房间可缓存版本，批量读 JSON。
4. `directory.rs:86` 起对所有公开房间逐个读取多个属性、构建/排序后才分页；limit 小也不降低总工作。评估目录投影、批量读取和稳定排序游标；同时修复过滤语义，避免仅优化错误结果。
5. 配置、DB pools、workers 使用进程全局状态；`MatrixServer` 明示每进程单实例、worker 跟随 runtime 结束。若需要嵌入或可重载生命周期，逐步引入持有任务句柄与取消信号的 service context，避免一次重构全部全局状态。
6. sync_v5、membership/invite、sync_v3、OIDC 等大模块同时承担协议、DB 和业务策略（含内嵌测试，不能仅凭行数判质量差）。先抽取可见性判定、游标、投递确认等共享不变量，减少 R01/R06 这类路径分叉；删除无调用旧安全实现或明确标记，避免“代码存在即以为生效”。

建议首批压测矩阵：小/大房间、用户多设备、共享房间数递增、慢/故障联邦目的地、突发 to-device、URL preview 失败洪峰、DB 短断连。记录吞吐、p50/p95/p99、event-loop lag、SQL 数/时间、pool/permit wait、队列最老年龄、RSS、CPU、恢复时间；固定机器/配置/数据规模。验收以相同基线无正确性回退、目标场景尾延迟和资源预算改善为准，测量后再定数值 SLO。

## 后续实施顺序

| 阶段 | 建议工作包 | 完成标准 |
|---|---|---|
| A：权限与完整性 | R01 状态快照；R02 媒体 reservation；R03 下载策略；R04 有效期 | 每项具备真实 HTTP/签名 peer 回归；覆盖历史合法行为，default/all-features 都通过 |
| B：核心投递 | R05 单层许可；R06 推送响应+重试一起修；R07 恢复与监督 | 容量 1/饱和可前进，网关/DB 故障恢复后无需重启，持久队列不丢 |
| C：协议与边界 | R08/R09/R10；O01/O02；CI 新增失败阻断 | 声明与行为一致、资源有界、关键故障可告警，安全回归纳入 required |
| D：规模与架构 | SQL/目录批量化、基准、生命周期与共享策略抽取 | 有可复现前后性能数据及生产恢复演练证据 |

A/B 可在独立代码路径上并行开发；发布前都应完成。R06 的两层修复必须联动。R02 的 schema/对象状态迁移与 R04 的历史签名兼容性需要单独设计升级和回滚步骤。仓库 `docs/UPGRADING.md` 已要求特定迁移协调停旧 writer，应继续遵循，不能假设任意改动均支持滚动升级。

每个工作包交付：最小复现、修复、不变量说明、回归、失败恢复证据、兼容性/迁移说明。后续尚需独立核查 notary 返回值身份/签名信任模型、完整 state-resolution 差分、备份恢复演练，以及所有广告能力与测试映射；这些不计入本轮确定漏洞数量。
