# 并发、数据一致性与性能评审

评审日期：2026-11-12。基线：`61b2ad7f02e216e6292116abfa3bcdd8e85cae25`。本报告为定向静态审查，未运行数据库、服务端、压测或 Rust 构建；以下“确认”表示代码路径确认，不代表生产事故复现。路径行号均对应此基线。P1 为优先修复的正确性风险，P2 为下一阶段可靠性与规模问题。

## 已确认的实现问题

### CP-01 · P1 · 管理员清理历史会删除仍在使用的 DAG 末端

- 证据：`crates/data/src/room/timeline.rs:80` 仅按房间、时间、非状态事件选择待删除集合；`:106` 起从 `event_forward_extremities` 删除匹配事件，`:148` 删除事件本身。没有排除当前 DAG tips，也未取得 `lock_event_append`。保留房间的管理接口 `crates/server/src/routing/admin/room.rs:605` 直接调用它。
- 触发：房间最新且唯一的 forward extremity 是普通消息；管理员清理时间晚于该消息。不同于删除整个房间，这一接口不禁止房间后续使用。
- 影响：`crates/server/src/event/pdu.rs:1231` 从剩余 forward extremities 构建下一事件的 `prev_events`，`:1269` 从这些事件计算 depth。清理掉所有 tips 后将构造空 `prev_events`、depth 为 1 的后续事件，可能被后续授权或其他 homeserver 拒绝。若只清理一个分支，则丢失该分支；与并发写入也缺少一致的隔离边界。具体最终 HTTP 错误需实服验证，但删掉活跃 DAG tips 的数据破坏路径明确。
- 建议：统一管理员 purge 与 retention 的图保留规则，至少保留所有 live forward extremities、必要图/状态元数据；`crates/server/src/retention.rs:128` 已实现保留全部 tips 的判断，可作为起点。删除内容和删除图身份应使用明确区分的语义。在同一数据库事务中取得与写入兼容的 room fence，再选择和清理。
- 验收：单 tip、多个远端 fork tips、并发本地发送三种夹具分别 purge；随后本地发送并与另一 homeserver 同步，验证 prev_events、depth、state resolution 及历史分页连续性。
- 置信度：高（最终服务端表现待集成复现）。

### CP-02 · P2 · purge 的分块删除没有限制内存和总事务体积

- 证据：`crates/data/src/room/timeline.rs:80-87` 先 `.load` 全部 `(event_id, sn)`，`:93` 再构建 ID 引用数组，`:104` 才每 500 个执行多表删除；所有块位于同一个事务。`crates/server/src/routing/admin/room.rs:549` 可传 `i64::MAX` 覆盖全部非状态历史。
- 影响：峰值 Rust 内存仍与清理事件总量线性相关。单事务的行锁、WAL 和回滚负担随总量增长，500 分块只限制单条 SQL 的参数数量；大房间管理员清理可能引发 OOM 或长时间数据库压力。未测量具体吞吐、锁等待或 OOM 阈值。
- 建议：按 `sn` keyset 分页选取并处理有界批次；实现可恢复的清理作业和进度游标。若业务必须保证整个操作原子性，则优先用数据库端集合操作减少客户端物化，并明确限制单作业规模；不要仅把一个事务拆小而不定义中断与重试语义。
- 验收：至少百万级事件夹具，记录批次内存上限、事务时长、WAL、正常发送/同步 p95；中途中断再恢复，确认没有悬空关联记录。
- 置信度：高（规模影响为结构分析，未实测）。

### CP-03 · P2 · 长轮询没有监视回执和 presence，且存在注册前的通知窗口

- 证据：`crates/server/src/watcher.rs:10-35` 和 `:47-75` 只比较 typing、inbox、本人 key change、membership、timeline、account data、可选 profile。没有 `event_receipts` 或 presence 流。`:112` 在进入 watch 后才读取 baseline；`:119-123` 每三秒查询一次，最长十次。`crates/server/src/routing/client/sync_v3.rs:63` 先生成空响应，`:78` 才调用 watch。`ROTATE.fire()` 的实际调用位于 `global.rs:325` 关机路径，回执更新未主动唤醒它。
- 触发与影响：长轮询期间只有另一个成员提交回执或 presence 变化，且没有其他被监视流变化，则客户端等到请求超时才收到变化；默认等待上限为 30 秒。即使是被监视的消息，在首次空响应计算完成后、watch baseline 读到之前提交，也会被纳入 baseline 而不触发提前返回。此问题为延迟，通常不是永久丢事件。
- 建议：先登记通知/取得流基线，再读取响应；空响应才等待，并根据已经捕获的 token 检查是否有新数据。监视集合应覆盖响应可出现的各类流；数据库多节点场景可用通知加持久流位置校验，保留低频轮询兜底。
- 验收：无消息房间中单独回执、单独 presence、在第一次读与 watch 之间设 barrier 插入消息；验证等待中的 sync 在规定延迟内返回。测量空闲设备数 N 下数据库查询率，当前轮询仅粗略下界约为 N/3 次每秒，还未计重新构造完整 sync 的成本。
- 置信度：高。

### CP-04 · P2 · 被 room filter 排除的房间仍执行完整 payload 读取

- 证据：`crates/server/src/sync_v3.rs:146-164` 对全部 joined rooms 顺序调用 `load_joined_room`，返回后才决定是否加入响应。`load_joined_room` 在 `:474` 计算 `deliver_room`，但并未提前返回：`:507` 读取 timeline，`:532` 起可执行 backfill，`:850` 读取 receipts，`:866` 读取 room account data。`deliver_room` 主要用于少数成员 lazy-load 行为，未隔离上述读取。
- 影响：用户明确只同步一个房间时，数据库工作仍随全部加入房间数增长；被过滤房间的 payload 读失败也可能使本次 sync 失败。顺序 await 进一步叠加查询延迟。这里只确认浪费工作，未声称已有具体延迟数值。
- 建议：将跨房间 device-list/share-membership 必需计算与房间响应 payload 分开；只对需返回的房间计算 timeline、receipts、backfill、account data。不能简单在循环顶部跳过房间，因为目前它还累积全局加密设备列表变化。随后再评估有上限的房间并发和 SQL 批量化。
- 验收：相同用户分别加入 1/100/1000 房间，filter 固定选择 1 个；比较 SQL 数量与 p95。过滤房间注入 payload 读取失败，目标房间应仍能同步；加密设备列表正确性须保留回归。
- 置信度：高。

### CP-05 · P2 · 回执查询和响应随历史回执数增长，缺少最新状态投影

- 证据：`crates/data/src/room/receipt.rs:80-93`、`:111-115` 每次插入新行；`:22-27` 按房间及下限读取所有历史记录，无上限和 SQL 最新状态去重；`:54-64` 按 event ID 构造输出，不能消除同一用户在多个历史事件上的旧回执。`crates/server/src/sync_v3.rs:582-586` 初始房间 sync 使用加入时的位置，`:850` 将其后的全部回执加载并编码。表主键为 `sn`，没有 `(room,user,type,thread)` 的最新记录约束（`crates/data/migrations/2025-06-05-063543_main/up.sql:724`）。
- 影响：长期成员换设备或重新初始同步时，会读取加入以来的回执历史；长时间离线后的增量同步也会读取整个窗口。增加数据库、进程内存和 JSON 响应成本。相同用户/事件/type 多次更新时，SQL 以 sn 降序读取，而后续 `.insert` 会让旧记录覆盖较新的记录；这也是当前聚合方式的正确性问题。
- 建议：保留用于增量/federation 游标的历史日志，同时维护受权限控制的最新 receipt 投影；按 `(room,user,type,thread)` 选最新值，再按 sync 窗口选择有变化的键。清理历史前确认 sending 游标与最慢消费者水位。私密回执权限问题由安全报告单列。
- 验收：单用户 100 万次不同事件回执，初始同步响应应与当前 receipt 键数相关；同一事件连续写入不同时间戳，输出必须保留最新值；多线程回执互不覆盖。
- 置信度：高。

## 需要补充复现的风险

### CP-R1 · MutexMap 等待取消后可能留下无使用者条目

`crates/server/src/utils/mutex_map.rs:40-50` 在等待前把锁保存在 map 中；唯一清理位于已取得 guard 的 `Drop`（`:131-135`）。可疑时序为：A 持锁，B 等待；A 释放时引用数使条目保留；B 被唤醒但尚未被 poll 就取消，此时没有 guard 的 Drop 清理。最后 map 只剩自身引用。若该 key 永不再访问，条目可能常驻。正常无等待释放时还对整个 HashMap 执行 `retain`，持有全局同步 mutex 扫描。建议使用带 key 的 RAII waiter 清理或经过审查的 keyed-lock 实现；先用手动 poll/取消构造确定性测试，避免依赖 sleep 概率测试。该项未运行复现，不计入已确认缺陷数。

## 已有设计值得保留

- 数据库连接的事务 lease 与取消回收逻辑集中在 `crates/data/src/pool.rs:31-50`、`:127` 起；测试文件覆盖取消后释放数据库锁。不能仅因为使用连接池就推断事务跨连接。
- 本地事件写入已有数据库 transaction-scoped room append fence（`crates/data/src/room/timeline.rs:27`），较进程内 mutex 更能覆盖多进程写入。purge 等旁路应接入相同一致性规则。
- 发送 worker 使用有界 wakeup channel、durable queue 扫描兜底、目的端 lifecycle 并发预算、失败重启和指数退避（`crates/server/src/sending/guard.rs:81`、`:127`、`:162`）；未发现“无界 spawn 每个事件”式发送架构。
- CPU 工作已封装 `spawn_blocking` 和有界许可（`crates/server/src/utils/cpu.rs`），媒体预览也有限流。这些局部治理不能替代 sync、purge 和数据库查询的端到端资源预算。

## 建议的改进顺序与量化方法

1. 先修 CP-01，加入实际数据库加并发写入/混合联邦的正确性测试。
2. 同期修复私密回执权限，再重构 CP-05 的最新状态投影和聚合顺序，避免性能优化固化权限错误。
3. 修复 CP-03 通知时序和流覆盖，先定义可验证的 sync 唤醒延迟目标。
4. 依次解决 CP-02、CP-04；记录 query/request、pool wait、transaction duration、RSS、队列深度/最老项年龄、sync p50/p95/p99、数据库 WAL 与锁等待。
5. 使用真实 PostgreSQL 与服务进程运行现有 `tests/performance_load.py`；固定 binary SHA、配置、数据量、机器和工作负载，并分初始/增量/空闲 sync、单热点/多房间发送、在线/故障远端分组。当前没有这些测量，本报告不提供虚构的性能评分或吞吐提升比例。


## 专项修复进展（本分支）

CP-01/02：purge 在每个最多 500 事件的事务内先取得 room append fence，再排除所有 forward tips；每批原子清理关联数据。已提交批次保留，失败/取消后重试同一 cutoff 即处理剩余集合；不是全房间原子操作，也没有后台作业 ID。时间戳直接校验为非负毫秒，避免 i64::MAX 经 SystemTime 转换溢出。

CP-03：v3/v5 在首次响应构造前捕获 watch baseline，等待前立即比较，加入可见回执与共享用户 presence。轮询仍为三秒间隔，不承诺实时 push；跨节点使用数据库流查询。

CP-04：过滤房间保留 captured frame 的成员初始集合/增量汇总，提前返回，跳过 timeline/backfill、receipt、account data；修复初始房间集合覆盖为 extend。

CP-05：通过 PostgreSQL DISTINCT ON 投影 user/type/JSON thread 键的最新回执，保留 append-only 日志和 federation 游标；新增匹配 expression 索引。read_receipts 同步采用安全分支 viewer 参数，禁止把他人私密回执加入投影。返回值规模随当前 receipt 键数增长，不再随同一键的历史更新次数增长；数据库仍需要处理历史索引，不声称 SQL 扫描时间与历史完全无关。额外的汇总表/日志回收策略需另行根据负载测量决定。

新增独立数据库回归：concurrency_review_tests 两项、watcher::review_tests 一项、sync_v3::resource_review_tests 一项；CI exact-name 列表在 default/all features 分别新建空库、独立进程运行。开发分支仅执行 rustfmt 和 git diff --check；编译、数据库及 HTTP 回归由主 agent 在原 workspace 原生 target 集中执行，结果应补入 PR。CP-R1 尚未复现，未改动 MutexMap。


### 交叉复核后的补充修复及限制

公共 `append_pdu`（联邦/直接 outlier promotion）现在在最终发布阶段取得与本地写入、purge 相同的数据库 append fence；本地 builder 已持有 fence 时调用私有 body，避免跨连接重复取同一锁。soft-fail 的 tip read/replace 同样取得该 fence。联邦网络验证、缺失状态抓取未被纳入新增 fence。forward tip 删除/插入改为单事务，purge 排除仍在验证/升级中的 outlier。新增 trigger+advisory barrier 回归检验暂停的 promotion、原 tip 集可见性及 purge 等待行为。

latest receipt 查询返回独立 event 向量，v3 可保留同一 event/type/user 的 unthreaded、main、thread 三种回执。v5 的现有 MSC3960 单事件 map wire 结构不能表达同一嵌套键的多个 thread 值，保持兼容格式；投影向量按 sn 升序输出，使 combine 的冲突明确选择最新 sn，不再按 thread 字典序覆盖。本分支不宣称 v5 能无损表达此特殊多 thread 情况。

watcher 修复注册时间窗口及 receipt/presence 覆盖，但 MAX(sn) 摘要仍不能检测低 sn 事务在高 sn 事务之后提交的情况。这需要对各流建立提交可见 publication 水位/fence；现阶段作为后续建设项，长轮询仍可能等到超时才返回这种迟提交更新。未宣称所有提交顺序竞态已消除。


wire 格式参考：[MSC3960 旧扩展](https://github.com/matrix-org/matrix-spec-proposals/blob/kegan/ssext-receipts/proposals/3960-sliding-sync-receipts.md) 定义每房间单 m.receipt；[MSC4551 后继扩展](https://github.com/matrix-org/matrix-spec-proposals/blob/main/proposals/4551-sliding-sync-extension-receipts.md) 仍定义每房间单 receipt content map，且移除了 type/content wrapper。当前项目采用旧 wrapper 的协议升级是独立后续项，本专项保持兼容。
