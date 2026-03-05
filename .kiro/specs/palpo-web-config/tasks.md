# 实施计划: Palpo Web配置管理

## 开发流程 (ADDS)

### 会话开始检查清单

每次开发会话开始前，必须完成以下检查：

1. **环境验证**
   ```bash
   bash init.sh
   ```
   - [ ] Rust 工具链正常
### 第一阶段：基础架构和UI页面框架
### 第二阶段：用户管理功能
### 第三阶段：房间和媒体管理功能
### 第四阶段：新模块（目录、联邦、令牌、举报、设备）

---

## 阶段一：基础架构和UI页面框架

### 任务组 1: 项目设置和基础架构 (已完成)

- [x] 1.1 验证项目结构和配置文件 (1小时)
  - **需求**: 所有需求的基础设施
  - **前置依赖**: 无
  - **实现内容**:
    - 验证Cargo.toml、Dioxus.toml、tailwind.config.js配置完整
    - 验证项目目录结构（src/api/, src/web/, assets/等）
    - 验证开发和构建脚本（dev.sh, build.sh）
  - **测试用例**:
    - [x] 所有配置文件存在且格式正确
    - [x] 目录结构符合设计文档
    - [x] 脚本可执行且权限正确
  - **完成标准**:
    - [x] 项目结构验证通过
    - [x] 配置文件验证通过
    - [x] 脚本验证通过
  - **验证命令**:
    ```bash
    ls -la Cargo.toml Dioxus.toml
    ls -la crates/admin-ui/src/
    ```

- [x] 1.2 设置开发和构建工具链 (1小时)
  - **需求**: 开发效率基础
  - **前置依赖**: 任务1.1
  - **实现内容**:
    - 安装和配置Dioxus CLI
    - 创建开发脚本和构建脚本
    - 配置热重载和WASM优化设置
  - **测试用例**:
    - [x] Dioxus CLI 安装成功
    - [x] 开发脚本可以启动开发服务器
    - [x] 构建脚本可以生成生产构建
  - **完成标准**:
    - [x] dx 命令可用
    - [x] 开发服务器可以启动
    - [x] 生产构建成功
  - **验证命令**:
    ```bash
    dx --version
    bash crates/admin-ui/scripts/dev.sh
    bash crates/admin-ui/scripts/build.sh
    ```

### 任务组 2: 核心数据模型和错误处理 (已完成)

- [x] 2.1 实现配置数据模型 (2小时)
  - **需求**: 12.1, 12.2, 12.3, 12.4, 12.5
  - **前置依赖**: 任务1.2
  - **实现内容**:
    - 创建ServerConfigSection、DatabaseConfigSection等配置结构体
    - 实现serde序列化和反序列化
    - 定义配置验证规则和约束
  - **测试用例**:
    - [x] 配置结构体可以序列化为JSON/TOML
    - [x] 配置结构体可以从JSON/TOML反序列化
    - [x] 配置验证规则正确工作
    - [x] 无效配置被正确拒绝
  - **完成标准**:
    - [x] 所有配置模型定义完成
    - [x] 序列化/反序列化测试通过
    - [x] 验证规则测试通过
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-ui config_model
    ```

- [x] 2.2 测试配置数据模型 (1小时)
  - **需求**: 12.1, 12.2, 12.3, 12.4, 12.5
  - **前置依赖**: 任务2.1
  - **实现内容**:
    - 编写配置序列化/反序列化单元测试
    - 编写配置验证规则单元测试
    - 编写配置验证属性测试（属性1: 配置验证一致性）
  - **测试用例**:
    - [x] 测试所有配置字段的序列化
    - [x] 测试所有配置字段的反序列化
    - [x] 测试边界值验证
    - [x] 测试无效输入拒绝
    - [x] 属性测试：序列化后反序列化应得到相同配置
  - **完成标准**:
    - [x] 单元测试覆盖率 > 90%
    - [x] 属性测试通过
    - [x] 提供测试执行输出
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-ui config_model -- --nocapture
    ```

- [x] 2.3 实现错误处理系统 (2小时)
  - **需求**: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6
  - **前置依赖**: 任务2.1
  - **实现内容**:
    - 定义WebConfigError错误类型
    - 实现错误转换和HTTP状态码映射
    - 创建前端ApiError类型和错误处理钩子
  - **测试用例**:
    - [x] 所有错误类型可以正确创建
    - [x] 错误转换正确工作
    - [x] HTTP状态码映射正确
    - [x] 前端错误处理钩子正常工作
  - **完成标准**:
    - [x] 错误类型定义完成
    - [x] 错误转换测试通过
    - [x] HTTP映射测试通过
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-ui error_handling
    ```

- [x] 2.4 测试错误处理系统 (1小时)
  - **需求**: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6
  - **前置依赖**: 任务2.3
  - **实现内容**:
    - 编写错误类型转换单元测试
    - 编写HTTP状态码映射单元测试
    - 编写错误处理钩子单元测试
  - **测试用例**:
    - [x] 测试所有错误类型的转换
    - [x] 测试所有HTTP状态码映射
    - [x] 测试错误消息格式化
    - [x] 测试前端错误处理流程
  - **完成标准**:
    - [x] 单元测试覆盖率 > 85%
    - [x] 所有测试通过
    - [x] 提供测试执行输出
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-ui error_handling -- --nocapture
    ```

### 任务组 3: 认证和授权中间件 (已完成)完成必须提供工具执行证据
4. **时间限制**: 每个任务应在 1-4 小时内完成

### 任务完成标准

每个任务完成时必须满足：

1. **测试证据**
   - 单元测试通过截图或输出
   - 集成测试通过截图或输出
   - 手动测试清单（前端功能）

2. **代码质量**
   - 无编译错误和警告
   - 通过 `cargo clippy` 检查
   - 代码有适当注释

3. **文档更新**
   - 更新 `progress.md` 标记任务完成
   - 记录遇到的问题和解决方案
   - 更新 API 文档（如有新增）

### 回归检查协议

在开始新功能开发前：

1. 运行完整测试套件
2. 确认所有测试通过
3. 如有失败，先修复回归问题
4. 记录回归问题到 `progress.md`

---

## 概述

本实施计划将Palpo Matrix服务器web管理界面分解为增量开发的任务。项目采用Rust + Salvo后端和Dioxus前端的全栈Rust架构，提供现代化的配置和运营管理体验。

技术栈：
- 后端：Rust + Salvo web框架 + PostgreSQL + Diesel ORM
- 前端：Dioxus (Rust WebAssembly框架) + TailwindCSS
- 测试：单元测试 + 集成测试 + 属性测试

## 实施阶段

### 第一阶段：基础架构和UI页面框架
### 第二阶段：用户管理功能
### 第三阶段：房间和媒体管理功能
### 第四阶段：新模块（目录、联邦、令牌、举报、设备）
- [ ] 所有 test_cases 状态为 passed
- [ ] 提供工具执行证据
- [ ] 代码已提交 Git (格式: `feat(模块): 描述 [Closes #任务ID]`)
- [ ] progress.md 已更新

### 回归检查
如果发现已完成功能被破坏:
1. 🛑 立即停止新功能开发
2. 🔧 优先修复回归问题
3. 📝 在 progress.md 记录回归详情

---

## 概述

本实施计划将Palpo Matrix服务器web管理界面分解为增量开发的任务。项目采用Rust + Salvo后端和Dioxus前端的全栈Rust架构，提供现代化的配置和运营管理体验。

技术栈：
- 后端：Rust + Salvo web框架 + PostgreSQL + Diesel ORM
- 前端：Dioxus (Rust WebAssembly框架) + TailwindCSS
- 测试：单元测试 + 集成测试 + 属性测试

## 实施阶段

### 第一阶段：基础架构和UI页面框架
### 第二阶段：用户管理功能
### 第三阶段：房间和媒体管理功能
### 第四阶段：新模块（目录、联邦、令牌、举报、设备）

---

## 阶段一：基础架构和UI页面框架

- [x] 1. 项目设置和基础架构
  - [x] 1.1 验证项目结构和配置文件
    - 验证Cargo.toml、Dioxus.toml、tailwind.config.js配置完整
    - 验证项目目录结构（src/api/, src/web/, assets/等）
    - 验证开发和构建脚本（dev.sh, build.sh）
    - _需求: 所有需求的基础设施_

  - [x] 1.2 设置开发和构建工具链
    - 安装和配置Dioxus CLI
    - 创建开发脚本和构建脚本
    - 配置热重载和WASM优化设置
    - _需求: 开发效率基础_

- [x] 2. 核心数据模型和错误处理
  - [x] 2.1 实现配置数据模型
    - 创建ServerConfigSection、DatabaseConfigSection等配置结构体
    - 实现serde序列化和反序列化
    - 定义配置验证规则和约束
    - _需求: 12.1, 12.2, 12.3, 12.4, 12.5_

  - [x] 2.2 测试配置数据模型
    - 编写配置序列化/反序列化单元测试
    - 编写配置验证规则单元测试
    - 编写配置验证属性测试（属性1: 配置验证一致性）
    - _需求: 12.1, 12.2, 12.3, 12.4, 12.5_

  - [x] 2.3 实现错误处理系统
    - 定义WebConfigError错误类型
    - 实现错误转换和HTTP状态码映射
    - 创建前端ApiError类型和错误处理钩子
    - _需求: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

  - [x] 2.4 测试错误处理系统
    - 编写错误类型转换单元测试
    - 编写HTTP状态码映射单元测试
    - 编写错误处理钩子单元测试
    - _需求: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

- [x] 3. 认证和授权中间件
  - [x] 3.1 实现认证中间件
    - 创建AuthMiddleware结构体
    - 实现JWT令牌验证和管理员权限检查
    - 实现会话管理和超时处理
    - _需求: 10.1, 10.2, 10.3, 10.4, 10.5_

  - [x] 3.2 测试认证中间件
    - 编写JWT验证单元测试
    - 编写权限检查单元测试
    - 编写认证属性测试（属性2: 认证和授权一致性）
    - 编写认证集成测试（登录、令牌刷新、会话管理）
    - _需求: 10.1, 10.2, 10.3, 10.4, 10.5_

  - [x] 3.3 实现审计日志系统
    - 创建AuditLogEntry数据模型
    - 实现审计日志记录中间件
    - 创建审计日志查询和管理API
    - _需求: 操作日志记录_

  - [x] 3.4 测试审计日志系统
    - 编写审计日志记录单元测试
    - 编写审计日志查询单元测试
    - 编写审计日志过滤单元测试
    - 编写审计日志集成测试
    - _需求: 操作日志记录_

- [x] 4. Dioxus前端基础架构
  - [x] 4.1 设置Dioxus应用结构
    - 创建主应用组件和路由配置
    - 实现认证路由保护和导航
    - 设置全局状态管理（AuthState枚举、AppState）
    - _需求: 11.1, 11.2, 11.3_

  - [x] 4.2 实现API客户端服务
    - 创建gloo-net + 拦截器模式的API客户端
    - 实现RequestConfig、HttpMethod、TokenManager
    - 实现AuthInterceptor、ErrorInterceptor
    - 实现令牌管理和自动刷新
    - _需求: 10.1, 10.2, 10.4_

  - [x] 4.3 测试API客户端服务
    - 编写API客户端单元测试（后端部分）
    - 编写令牌管理单元测试
    - 编写拦截器单元测试
    - _需求: 10.1, 10.2, 10.4_

- [x] 5. 通用UI组件
  - [x] 5.1 实现通用UI组件
    - 创建表单组件（输入框、选择器、按钮）
    - 实现错误消息和验证反馈组件
    - 创建加载指示器和进度条组件
    - 实现确认对话框组件
    - _需求: 11.4, 11.5, 11.6_

  - [x] 5.2 实现布局和导航组件
    - 创建管理界面布局（侧边栏、主内容区）
    - 实现响应式导航菜单
    - 创建面包屑导航和页面标题
    - 实现主题切换（深色/浅色模式）
    - _需求: 11.1, 11.7_

  - [x] 5.3 手动测试UI组件
    - 创建UI组件测试清单（表单、反馈、加载、布局）
    - 启动开发服务器进行手动测试
    - 记录测试结果
    - _需求: 11.4, 11.5, 11.6, 11.7_

- [x] 6. 配置管理前端页面
  - [x] 6.1 实现配置表单页面
    - 创建分组的配置表单（服务器、数据库、联邦等）
    - 实现实时验证和错误提示
    - 实现配置保存和重置功能
    - _需求: 12.1, 12.2, 12.3, 12.4, 12.5_

  - [x] 6.2 实现配置模板页面
    - 创建模板选择和应用界面
    - 实现模板创建和编辑功能
    - 实现模板预览和差异对比
    - _需求: 配置模板功能_

  - [x] 6.3 实现配置导入导出页面
    - 创建配置导出界面（格式选择、选项配置）
    - 实现配置导入界面（文件上传、预览、冲突解决）
    - 实现导入导出历史记录
    - _需求: 配置导入导出功能_

  - [x] 6.4 测试配置管理功能
    - 编写数据验证逻辑单元测试（服务器名称、IP、数据库连接字符串）
    - 编写数据转换逻辑单元测试（TOML/YAML/JSON格式转换）
    - 编写配置管理集成测试（读取、写入、验证、模板应用、导入导出）
    - 手动测试配置管理页面（创建测试清单并执行）
    - _需求: 11.4, 11.5, 12.x_

- [ ] 7. 用户管理前端页面（占位符→框架）
  - [ ] 7.1 实现用户列表页面框架
    - 创建用户列表组件，支持分页和排序
    - 实现用户搜索和过滤功能
    - 实现批量操作下拉菜单
    - _需求: 1.1, 1.2, 1.3, 1.4, 1.5, 1.18_

  - [ ] 7.2 实现用户详情页面框架
    - 创建用户详情标签页（基本信息、权限、设备、连接等）
    - 实现用户编辑表单
    - 实现用户锁定/解锁、停用/激活功能
    - _需求: 1.5, 1.6, 1.7, 1.8, 1.9_

- [ ] 8. 房间管理前端页面（占位符→框架）
  - [ ] 8.1 实现房间列表页面框架
    - 创建房间列表组件，支持分页和排序
    - 实现房间搜索和过滤（公开房间、空房间）
    - 实现批量操作下拉菜单
    - _需求: 2.1, 2.2, 2.11_

  - [ ] 8.2 实现房间详情页面框架
    - 创建房间详情标签页（基本信息、成员、状态、媒体等）
    - 实现房间编辑表单
    - 实现房间删除和封禁功能
    - _需求: 2.2, 2.3, 2.4, 2.5, 2.6, 2.10_

- [ ] 9. 媒体管理前端页面（占位符→框架）
  - [ ] 9.1 实现媒体管理页面框架
    - 创建媒体文件列表组件
    - 实现媒体搜索和过滤功能
    - 实现批量删除功能
    - _需求: 3.1, 3.2, 3.3, 3.4, 3.5_

  - [ ] 9.2 实现用户媒体统计页面框架
    - 创建用户媒体统计列表
    - 实现按用户搜索功能
    - 实现批量操作功能
    - _需求: 8.1, 8.2, 8.3_

- [ ] 10. 联邦管理前端页面（占位符→框架）
  - [ ] 10.1 实现联邦目的地页面框架
    - 创建联邦目的地列表组件
    - 实现目的地搜索功能
    - 实现连接状态显示和重连功能
    - _需求: 5.1, 5.2, 5.3, 5.4, 5.5_

- [ ] 11. Appservice管理前端页面（占位符→框架）
  - [ ] 11.1 实现Appservice管理页面框架
    - 创建Appservice列表组件
    - 实现Appservice注册表单（YAML上传）
    - 实现Appservice测试和状态监控
    - _需求: 15.1, 15.2, 15.3, 15.4, 15.5_

- [ ] 12. 服务器控制前端页面（占位符→框架）
  - [ ] 12.1 实现服务器状态页面框架
    - 创建服务器状态仪表板
    - 实现服务器通知显示
    - _需求: 13.1, 13.2, 13.3, 13.4_

  - [ ] 12.2 实现服务器命令页面框架
    - 创建命令列表组件
    - 实现命令执行界面
    - 实现定时命令管理
    - _需求: 14.1, 14.2, 14.3, 14.4_

- [ ] 13. 检查点 - 阶段一完成
  - 所有UI页面框架实现完成
  - 基础架构和认证系统就绪
  - 询问用户是否有问题

---

## 阶段二：用户管理功能

### 任务组 14: 用户管理后端API

- [ ] 14.1 实现用户名可用性检查API (1.5小时)
  - **需求**: 1.3
  - **前置依赖**: 任务3.1 (认证中间件)
  - **实现内容**:
    - 创建 check_username_availability 端点
    - 实现用户名格式验证
    - 实现数据库查询检查
  - **测试用例**:
    - [ ] 测试有效用户名返回 available: true (输入: "newuser" -> 输出: {"available": true})
    - [ ] 测试已存在用户名返回 available: false (输入: "admin" -> 输出: {"available": false})
    - [ ] 测试无效格式用户名返回验证错误 (输入: "a" -> 输出: 400错误 "用户名长度至少3字符")
    - [ ] 测试特殊字符处理 (输入: "user@#$" -> 输出: 400错误 "用户名只能包含字母数字和下划线")
    - [ ] 测试长度边界 (输入: "ab" -> 错误, "abc" -> 成功, "a"*256 -> 错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server username_availability -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.2 实现用户锁定/解锁功能 (2小时)
  - **需求**: 1.5
  - **前置依赖**: 任务14.1
  - **实现内容**:
    - 创建 lock_user 和 unlock_user 端点
    - 实现数据库状态更新
    - 实现审计日志记录
  - **测试用例**:
    - [ ] 测试锁定用户成功 (输入: user_id="@test:example.com" -> 输出: {"locked": true})
    - [ ] 测试解锁用户成功 (输入: user_id="@test:example.com" -> 输出: {"locked": false})
    - [ ] 测试锁定已锁定用户（幂等性） (输入: 已锁定用户 -> 输出: 200 OK, 状态不变)
    - [ ] 测试解锁未锁定用户（幂等性） (输入: 未锁定用户 -> 输出: 200 OK, 状态不变)
    - [ ] 测试锁定不存在的用户返回404 (输入: "@nonexistent:example.com" -> 输出: 404错误)
    - [ ] 测试审计日志正确记录 (验证: audit_log表包含操作记录)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 审计日志测试通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server user_lock -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.3 实现用户暂停功能 (MSC3823) (2小时)
  - **需求**: 1.5
  - **前置依赖**: 任务14.2
  - **实现内容**:
    - 创建 suspend_user 和 unsuspend_user 端点
    - 实现暂停原因记录
    - 实现暂停状态查询
  - **测试用例**:
    - [ ] 测试暂停用户成功 (输入: user_id + reason="违规行为" -> 输出: {"suspended": true, "reason": "违规行为"})
    - [ ] 测试取消暂停用户成功 (输入: user_id -> 输出: {"suspended": false})
    - [ ] 测试暂停原因正确记录 (验证: 数据库包含暂停原因)
    - [ ] 测试暂停状态查询 (输入: user_id -> 输出: 暂停状态和原因)
    - [ ] 测试暂停已停用用户返回错误 (输入: 已停用用户 -> 输出: 400错误 "无法暂停已停用用户")
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server user_suspend -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.4 实现设备管理API (2小时)
  - **需求**: 9.1, 9.2, 9.3
  - **前置依赖**: 任务14.3
  - **实现内容**:
    - 创建设备管理API（DeviceAdminAPI）
    - 实现设备列表查询
    - 实现设备删除功能
  - **测试用例**:
    - [ ] 测试获取用户设备列表 (输入: user_id -> 输出: 设备列表数组)
    - [ ] 测试删除单个设备 (输入: user_id + device_id -> 输出: 200 OK)
    - [ ] 测试删除不存在的设备 (输入: 无效device_id -> 输出: 404错误)
    - [ ] 测试删除其他用户的设备需要管理员权限 (输入: 非管理员 -> 输出: 403错误)
    - [ ] 测试批量删除设备 (输入: user_id + device_ids[] -> 输出: 删除计数)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server device_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.5 实现连接管理API (1.5小时)
  - **需求**: 1.8
  - **前置依赖**: 任务14.4
  - **实现内容**:
    - 创建连接管理API
    - 实现连接信息查询功能
  - **测试用例**:
    - [ ] 测试获取用户连接列表 (输入: user_id -> 输出: 连接信息数组)
    - [ ] 测试连接信息包含IP地址和时间戳 (验证: 输出包含ip, last_seen字段)
    - [ ] 测试按时间范围过滤连接 (输入: start_time, end_time -> 输出: 过滤后的连接)
    - [ ] 测试分页查询连接 (输入: page=1, size=10 -> 输出: 10条记录)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server connection_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.6 实现Pushers管理API (1.5小时)
  - **需求**: 1.9
  - **前置依赖**: 任务14.5
  - **实现内容**:
    - 创建pushers管理API
    - 实现pushers列表功能
  - **测试用例**:
    - [ ] 测试获取用户pushers列表 (输入: user_id -> 输出: pushers数组)
    - [ ] 测试pusher信息包含类型和配置 (验证: 输出包含kind, app_id, data字段)
    - [ ] 测试删除pusher (输入: user_id + pusher_key -> 输出: 200 OK)
    - [ ] 测试获取不存在用户的pushers (输入: 无效user_id -> 输出: 404错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server pusher_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.7 实现成员资格管理API (1.5小时)
  - **需求**: 1.12
  - **前置依赖**: 任务14.6
  - **实现内容**:
    - 创建成员资格管理API
    - 实现成员资格列表功能
  - **测试用例**:
    - [ ] 测试获取用户房间成员资格列表 (输入: user_id -> 输出: 房间列表)
    - [ ] 测试成员资格信息包含房间ID和角色 (验证: 输出包含room_id, membership字段)
    - [ ] 测试按成员资格类型过滤 (输入: membership="join" -> 输出: 仅已加入的房间)
    - [ ] 测试分页查询成员资格 (输入: page=1, size=20 -> 输出: 20条记录)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server membership_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.8 实现速率限制管理API (2小时)
  - **需求**: 1.13
  - **前置依赖**: 任务14.7
  - **实现内容**:
    - 创建速率限制管理API
    - 实现速率限制配置功能
  - **测试用例**:
    - [ ] 测试获取用户速率限制配置 (输入: user_id -> 输出: 速率限制配置)
    - [ ] 测试设置速率限制 (输入: user_id + rate_config -> 输出: 200 OK)
    - [ ] 测试重置速率限制为默认值 (输入: user_id -> 输出: 默认配置)
    - [ ] 测试无效速率限制值被拒绝 (输入: 负数或0 -> 输出: 400错误)
    - [ ] 测试速率限制立即生效 (验证: 设置后立即应用)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server rate_limit_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.9 实现实验功能管理API (1.5小时)
  - **需求**: 1.14
  - **前置依赖**: 任务14.8
  - **实现内容**:
    - 创建实验功能管理API
    - 实现实验功能配置功能
  - **测试用例**:
    - [ ] 测试获取用户实验功能列表 (输入: user_id -> 输出: 功能列表)
    - [ ] 测试启用实验功能 (输入: user_id + feature_name -> 输出: 200 OK)
    - [ ] 测试禁用实验功能 (输入: user_id + feature_name -> 输出: 200 OK)
    - [ ] 测试启用不存在的功能返回错误 (输入: 无效feature -> 输出: 404错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server experimental_features -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.10 实现账户数据管理API (2小时)
  - **需求**: 1.15
  - **前置依赖**: 任务14.9
  - **实现内容**:
    - 创建账户数据管理API
    - 实现账户数据查看和编辑功能
  - **测试用例**:
    - [ ] 测试获取用户账户数据 (输入: user_id + data_type -> 输出: 账户数据JSON)
    - [ ] 测试更新账户数据 (输入: user_id + data -> 输出: 200 OK)
    - [ ] 测试删除账户数据 (输入: user_id + data_type -> 输出: 200 OK)
    - [ ] 测试无效JSON格式被拒绝 (输入: 无效JSON -> 输出: 400错误)
    - [ ] 测试账户数据大小限制 (输入: 超大数据 -> 输出: 413错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server account_data_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.11 实现第三方标识管理API (2小时)
  - **需求**: 1.16
  - **前置依赖**: 任务14.10
  - **实现内容**:
    - 创建第三方标识管理API
    - 实现threepids列表和添加功能
  - **测试用例**:
    - [ ] 测试获取用户threepids列表 (输入: user_id -> 输出: threepids数组)
    - [ ] 测试添加email threepid (输入: user_id + email -> 输出: 200 OK)
    - [ ] 测试添加msisdn threepid (输入: user_id + phone -> 输出: 200 OK)
    - [ ] 测试删除threepid (输入: user_id + medium + address -> 输出: 200 OK)
    - [ ] 测试添加重复threepid被拒绝 (输入: 已存在的email -> 输出: 409错误)
    - [ ] 测试无效email格式被拒绝 (输入: "invalid-email" -> 输出: 400错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server threepid_admin -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.12 实现SSO外部ID管理API (1.5小时)
  - **需求**: 1.17
  - **前置依赖**: 任务14.11
  - **实现内容**:
    - 创建SSO外部ID管理API
    - 实现外部ID管理功能
  - **测试用例**:
    - [ ] 测试获取用户外部ID列表 (输入: user_id -> 输出: 外部ID数组)
    - [ ] 测试添加外部ID (输入: user_id + auth_provider + external_id -> 输出: 200 OK)
    - [ ] 测试删除外部ID (输入: user_id + auth_provider + external_id -> 输出: 200 OK)
    - [ ] 测试添加重复外部ID被拒绝 (输入: 已存在的external_id -> 输出: 409错误)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server sso_external_id -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.13 实现批量用户注册API (2.5小时)
  - **需求**: 17.1, 17.2, 17.3, 17.4
  - **前置依赖**: 任务14.12
  - **实现内容**:
    - 创建批量注册API
    - 实现CSV解析和用户创建
    - 支持第三方标识
  - **测试用例**:
    - [ ] 测试CSV格式解析 (输入: 有效CSV -> 输出: 用户数据数组)
    - [ ] 测试批量创建用户 (输入: 用户数据数组 -> 输出: 创建结果)
    - [ ] 测试包含threepids的批量注册 (输入: CSV含email列 -> 输出: 用户含threepid)
    - [ ] 测试部分失败处理 (输入: 部分无效数据 -> 输出: 成功和失败列表)
    - [ ] 测试CSV格式错误处理 (输入: 无效CSV -> 输出: 400错误)
    - [ ] 测试批量操作事务性 (验证: 失败时回滚)
  - **完成标准**:
    - [ ] API端点实现完成
    - [ ] 单元测试覆盖率 > 90%
    - [ ] 所有测试用例通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server batch_registration -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.14 用户管理API属性测试 (2小时)
  - **需求**: 1.x, 9.x, 17.x
  - **前置依赖**: 任务14.1-14.13
  - **实现内容**:
    - 编写列表操作属性测试（属性3: 列表管理操作不变性）
    - 编写批量操作属性测试（属性4: 批量操作原子性）
  - **测试用例**:
    - [ ] 属性测试：分页查询一致性 (任意page/size -> 结果一致)
    - [ ] 属性测试：排序稳定性 (相同排序条件 -> 相同顺序)
    - [ ] 属性测试：批量操作原子性 (部分失败 -> 全部回滚)
    - [ ] 属性测试：并发操作安全性 (并发修改 -> 数据一致)
  - **完成标准**:
    - [ ] 属性测试实现完成
    - [ ] 所有属性测试通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server user_admin_properties -- --nocapture --ignored
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.15 用户管理API集成测试 (2.5小时)
  - **需求**: 1.x, 9.x, 17.x
  - **前置依赖**: 任务14.14
  - **实现内容**:
    - 编写用户完整生命周期集成测试
    - 编写权限变更集成测试
    - 编写设备管理集成测试
    - 编写批量操作集成测试
  - **测试用例**:
    - [ ] 集成测试：用户创建->编辑->锁定->解锁->停用完整流程
    - [ ] 集成测试：用户权限变更影响API访问
    - [ ] 集成测试：设备管理完整流程（列表->删除->验证）
    - [ ] 集成测试：批量用户注册完整流程（上传CSV->解析->创建->验证）
    - [ ] 集成测试：审计日志正确记录所有操作
  - **完成标准**:
    - [ ] 集成测试实现完成
    - [ ] 所有集成测试通过
    - [ ] 代码通过 clippy 检查（无警告）
    - [ ] 提供 cargo test 输出截图
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server --test user_admin_integration -- --nocapture
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14.16 用户管理API回归测试检查点 (1小时)
  - **需求**: 1.x, 9.x, 17.x
  - **前置依赖**: 任务14.1-14.15
  - **实现内容**:
    - 运行所有用户管理相关测试
    - 验证API端点集成
    - 检查性能指标
  - **测试用例**:
    - [ ] 所有单元测试通过（覆盖率 > 90%）
    - [ ] 所有属性测试通过
    - [ ] 所有集成测试通过
    - [ ] API响应时间 < 100ms (p95)
    - [ ] 无内存泄漏
  - **完成标准**:
    - [ ] 所有测试通过
    - [ ] 无编译警告
    - [ ] 性能指标达标
    - [ ] 提供完整测试报告
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-server user_admin
    cargo test --package palpo-admin-server --test '*' | grep user
    cargo clippy --package palpo-admin-server -- -D warnings
    ```

- [ ] 14. 用户管理后端API
  - [ ] 14.1 扩展UserAdminAPI
    - 实现用户名可用性检查
    - 实现用户锁定/解锁功能
    - 实现用户暂停功能（MSC3823）
    - 实现用户数据擦除功能
    - _需求: 1.3, 1.5_

  - [ ] 14.2 实现设备管理API
    - 创建设备管理API（DeviceAdminAPI）
    - 实现设备列表、删除功能
    - _需求: 9.1, 9.2, 9.3_

  - [ ] 14.3 实现连接管理API
    - 创建连接管理API
    - 实现连接信息查询功能
    - _需求: 1.8_

  - [ ] 14.4 实现推pushers管理API
    - 创建pushers管理API
    - 实现pushers列表功能
    - _需求: 1.9_

  - [ ] 14.5 实现成员资格管理API
    - 创建成员资格管理API
    - 实现成员资格列表功能
    - _需求: 1.12_

  - [ ] 14.6 实现速率限制管理API
    - 创建速率限制管理API
    - 实现速率限制配置功能
    - _需求: 1.13_

  - [ ] 14.7 实现实验功能管理API
    - 创建实验功能管理API
    - 实现实验功能配置功能
    - _需求: 1.14_

  - [ ] 14.8 实现账户数据管理API
    - 创建账户数据管理API
    - 实现账户数据查看和编辑功能
    - _需求: 1.15_

  - [ ] 14.9 实现第三方标识管理API
    - 创建第三方标识管理API
    - 实现threepids列表和添加功能
    - _需求: 1.16_

  - [ ] 14.10 实现SSO外部ID管理API
    - 创建SSO外部ID管理API
    - 实现外部ID管理功能
    - _需求: 1.17_

  - [ ] 14.11 实现批量用户注册API
    - 创建批量注册API
    - 实现CSV解析和用户创建
    - 支持第三方标识
    - _需求: 17.1, 17.2, 17.3, 17.4_

  - [ ] 14.12 测试用户管理API
    - 编写用户锁定/解锁单元测试
    - 编写用户暂停功能单元测试
    - 编写设备管理API单元测试
    - 编写连接管理API单元测试
    - 编写pushers管理API单元测试
    - 编写成员资格管理API单元测试
    - 编写速率限制管理API单元测试
    - 编写实验功能管理API单元测试
    - 编写账户数据管理API单元测试
    - 编写第三方标识管理API单元测试
    - 编写SSO外部ID管理API单元测试
    - 编写批量用户注册API单元测试
    - 编写列表操作属性测试（属性3: 列表管理操作不变性）
    - 编写批量操作属性测试（属性4: 批量操作原子性）
    - 编写用户管理集成测试（完整生命周期、权限变更、设备管理、批量操作）
    - _需求: 1.x, 9.x, 17.x_

### 任务组 15: 用户管理前端功能完善

- [ ] 15.1 完善用户列表功能 (3小时)
  - **需求**: 1.3, 1.4, 1.18
  - **前置依赖**: 任务14.16, 任务7.1
  - **实现内容**:
    - 实现用户名可用性实时检查
    - 实现密码生成器
    - 实现批量操作（服务器通知、删除）
  - **测试用例**:
    - [ ] 测试用户名输入时实时检查可用性 (输入: "newuser" -> 显示: "✓ 可用")
    - [ ] 测试已存在用户名显示不可用 (输入: "admin" -> 显示: "✗ 已被使用")
    - [ ] 测试密码生成器生成强密码 (点击生成 -> 输出: 16字符随机密码)
    - [ ] 测试批量选择用户 (选择3个用户 -> 显示: "已选择 3 个用户")
    - [ ] 测试批量发送服务器通知 (选择用户 + 输入通知 -> 显示: "通知已发送")
    - [ ] 测试批量删除用户确认对话框 (点击删除 -> 显示: 确认对话框)
  - **完成标准**:
    - [ ] 组件实现完成
    - [ ] 业务逻辑单元测试通过（用户名验证、密码生成）
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图
    - [ ] 更新 progress.md
  - **手动测试清单**:
    - [ ] 在浏览器中打开用户列表页面
    - [ ] 测试用户名可用性检查（输入新用户名和已存在用户名）
    - [ ] 测试密码生成器（点击生成按钮，验证密码强度）
    - [ ] 测试批量选择（全选、部分选择、取消选择）
    - [ ] 测试批量发送通知（选择用户、输入通知内容、发送）
    - [ ] 测试批量删除（选择用户、确认删除）
  - **验证命令**:
    ```bash
    # 启动开发服务器
    cd crates/admin-ui && dx serve
    # 访问 http://localhost:8080/users
    ```

- [ ] 15.2 完善用户详情功能 (3小时)
  - **需求**: 1.7, 1.8, 1.9
  - **前置依赖**: 任务15.1, 任务7.2
  - **实现内容**:
    - 实现设备管理标签页
    - 实现连接信息标签页
    - 实现pushers标签页
  - **测试用例**:
    - [ ] 测试设备列表显示 (打开标签页 -> 显示: 设备列表)
    - [ ] 测试删除设备功能 (点击删除 -> 显示: 确认对话框 -> 设备被删除)
    - [ ] 测试连接信息显示 (打开标签页 -> 显示: IP地址、时间戳)
    - [ ] 测试连接信息分页 (点击下一页 -> 显示: 下一页数据)
    - [ ] 测试pushers列表显示 (打开标签页 -> 显示: pusher类型和配置)
    - [ ] 测试删除pusher功能 (点击删除 -> pusher被删除)
  - **完成标准**:
    - [ ] 组件实现完成
    - [ ] 业务逻辑单元测试通过
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图
    - [ ] 更新 progress.md
  - **手动测试清单**:
    - [ ] 打开用户详情页面
    - [ ] 测试设备管理标签页（查看设备列表、删除设备）
    - [ ] 测试连接信息标签页（查看连接列表、分页）
    - [ ] 测试pushers标签页（查看pushers列表、删除pusher）
    - [ ] 测试标签页切换（在不同标签页之间切换）
  - **验证命令**:
    ```bash
    cd crates/admin-ui && dx serve
    # 访问 http://localhost:8080/users/@test:example.com
    ```

- [ ] 15.3 完善用户高级功能 (2.5小时)
  - **需求**: 1.12, 1.13, 1.14
  - **前置依赖**: 任务15.2
  - **实现内容**:
    - 实现成员资格标签页
    - 实现速率限制配置
    - 实现实验功能配置
  - **测试用例**:
    - [ ] 测试成员资格列表显示 (打开标签页 -> 显示: 房间列表)
    - [ ] 测试按成员资格类型过滤 (选择"已加入" -> 显示: 仅已加入的房间)
    - [ ] 测试速率限制配置显示 (打开配置 -> 显示: 当前限制值)
    - [ ] 测试修改速率限制 (输入新值 -> 保存 -> 显示: "保存成功")
    - [ ] 测试实验功能列表显示 (打开配置 -> 显示: 功能列表)
    - [ ] 测试启用/禁用实验功能 (切换开关 -> 状态改变)
  - **完成标准**:
    - [ ] 组件实现完成
    - [ ] 业务逻辑单元测试通过
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图
    - [ ] 更新 progress.md
  - **手动测试清单**:
    - [ ] 测试成员资格标签页（查看房间列表、过滤）
    - [ ] 测试速率限制配置（查看当前值、修改、保存）
    - [ ] 测试实验功能配置（查看功能列表、启用/禁用）
  - **验证命令**:
    ```bash
    cd crates/admin-ui && dx serve
    # 访问 http://localhost:8080/users/@test:example.com
    ```

- [ ] 15.4 完善用户账户数据功能 (2.5小时)
  - **需求**: 1.15, 1.16, 1.17
  - **前置依赖**: 任务15.3
  - **实现内容**:
    - 实现账户数据标签页
    - 实现第三方标识管理
    - 实现SSO外部ID管理
  - **测试用例**:
    - [ ] 测试账户数据显示 (打开标签页 -> 显示: JSON格式数据)
    - [ ] 测试编辑账户数据 (修改JSON -> 保存 -> 显示: "保存成功")
    - [ ] 测试JSON格式验证 (输入无效JSON -> 显示: 错误提示)
    - [ ] 测试threepids列表显示 (打开标签页 -> 显示: email/phone列表)
    - [ ] 测试添加threepid (输入email -> 添加 -> 显示在列表中)
    - [ ] 测试删除threepid (点击删除 -> 确认 -> 从列表移除)
    - [ ] 测试外部ID列表显示 (打开标签页 -> 显示: 外部ID列表)
    - [ ] 测试添加外部ID (输入provider和ID -> 添加 -> 显示在列表中)
  - **完成标准**:
    - [ ] 组件实现完成
    - [ ] 业务逻辑单元测试通过（JSON验证、email验证）
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图
    - [ ] 更新 progress.md
  - **手动测试清单**:
    - [ ] 测试账户数据标签页（查看数据、编辑、保存、验证）
    - [ ] 测试第三方标识管理（查看列表、添加email、添加phone、删除）
    - [ ] 测试SSO外部ID管理（查看列表、添加、删除）
  - **验证命令**:
    ```bash
    cd crates/admin-ui && dx serve
    # 访问 http://localhost:8080/users/@test:example.com
    ```

- [ ] 15.5 实现批量用户注册页面 (3小时)
  - **需求**: 17.1, 17.2, 17.3, 17.4
  - **前置依赖**: 任务15.4
  - **实现内容**:
    - 创建CSV上传界面
    - 实现导入预览和验证
    - 显示导入结果
  - **测试用例**:
    - [ ] 测试CSV文件上传 (选择文件 -> 显示: 文件名和大小)
    - [ ] 测试CSV格式验证 (上传无效CSV -> 显示: 格式错误)
    - [ ] 测试导入预览 (上传有效CSV -> 显示: 用户数据预览表格)
    - [ ] 测试数据验证 (预览时 -> 显示: 验证错误标记)
    - [ ] 测试确认导入 (点击导入 -> 显示: 进度条)
    - [ ] 测试导入结果显示 (导入完成 -> 显示: 成功/失败统计)
    - [ ] 测试下载错误报告 (有失败记录 -> 点击下载 -> 下载CSV)
  - **完成标准**:
    - [ ] 组件实现完成
    - [ ] 业务逻辑单元测试通过（CSV解析、数据验证）
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图
    - [ ] 更新 progress.md
  - **手动测试清单**:
    - [ ] 测试CSV文件上传（选择文件、拖拽文件）
    - [ ] 测试CSV格式验证（上传有效和无效CSV）
    - [ ] 测试导入预览（查看数据表格、验证错误标记）
    - [ ] 测试确认导入（点击导入、查看进度）
    - [ ] 测试导入结果（查看成功/失败统计、下载错误报告）
  - **验证命令**:
    ```bash
    cd crates/admin-ui && dx serve
    # 访问 http://localhost:8080/users/batch-register
    ```

- [ ] 15.6 测试用户管理前端功能 (2小时)
  - **需求**: 1.x, 17.x
  - **前置依赖**: 任务15.5
  - **实现内容**:
    - 编写用户搜索和过滤逻辑单元测试
    - 手动测试用户管理页面（创建测试清单并执行）
    - 可选：编写用户管理E2E测试
  - **测试用例**:
    - [ ] 单元测试：用户名搜索逻辑 (输入: "alice" -> 输出: 包含"alice"的用户)
    - [ ] 单元测试：用户状态过滤逻辑 (输入: "已锁定" -> 输出: 仅已锁定用户)
    - [ ] 单元测试：用户排序逻辑 (输入: "按创建时间" -> 输出: 按时间排序)
    - [ ] 手动测试：完整用户管理流程（创建、编辑、搜索、批量操作）
    - [ ] 可选E2E测试：用户创建流程自动化测试
  - **完成标准**:
    - [ ] 单元测试覆盖率 > 80%（可测试部分）
    - [ ] 所有单元测试通过
    - [ ] 手动测试清单完成
    - [ ] 提供测试截图和报告
    - [ ] 更新 progress.md
  - **验证命令**:
    ```bash
    cargo test --package palpo-admin-ui user_management_logic -- --nocapture
    cd crates/admin-ui && dx serve
    # 手动测试 http://localhost:8080/users
    ```

- [ ] 16. 检查点 - 阶段二完成
  - 用户管理所有功能实现完成
  - 设备、连接、pushers等功能就绪
  - 批量用户注册功能就绪
  - 询问用户是否有问题

---

## 阶段三：房间和媒体管理功能

- [ ] 17. 房间管理后端API
  - [ ] 17.1 扩展RoomAdminAPI
    - 实现房间状态事件列表
    - 实现房间前沿终点查询
    - 实现房间目录发布/取消发布
    - 实现房间管理员设置
    - _需求: 2.4, 2.6, 2.7, 2.8, 2.9_

  - [ ] 17.2 实现房间媒体API
    - 创建房间媒体API
    - 实现房间媒体列表功能
    - _需求: 2.5_

  - [ ] 17.3 测试房间管理API
    - 编写房间状态事件列表单元测试
    - 编写房间前沿终点查询单元测试
    - 编写房间目录发布/取消发布单元测试
    - 编写房间管理员设置单元测试
    - 编写房间媒体API单元测试
    - 编写批量发布/取消发布单元测试
    - 编写房间管理集成测试（完整生命周期、成员管理、目录发布、删除封禁）
    - _需求: 2.x, 4.x_

- [ ] 18. 媒体管理后端API
  - [ ] 18.1 扩展MediaAdminAPI
    - 实现用户媒体统计查询
    - 实现媒体隔离功能
    - 实现媒体保护功能
    - 实现远程媒体清理
    - _需求: 3.1, 3.3, 3.4, 3.5_

  - [ ] 18.2 实现用户媒体API
    - 创建用户媒体API
    - 实现用户媒体列表功能
    - _需求: 3.6_

  - [ ] 18.3 测试媒体管理API
    - 编写用户媒体统计查询单元测试
    - 编写媒体隔离功能单元测试
    - 编写媒体保护功能单元测试
    - 编写远程媒体清理单元测试
    - 编写用户媒体API单元测试
    - 编写媒体管理集成测试（上传、删除、隔离、保护、清理、统计）
    - _需求: 3.x, 8.x_

- [ ] 19. 房间管理前端功能完善
  - [ ] 19.1 完善房间列表功能
    - 实现公开房间和空房间过滤
    - 实现批量发布/取消发布
    - _需求: 2.11, 2.7, 2.8_

  - [ ] 19.2 完善房间详情功能
    - 实现状态事件标签页
    - 实现前沿终点标签页
    - 实现房间媒体标签页
    - 实现房间管理员管理
    - _需求: 2.4, 2.6, 2.5, 2.9_

  - [ ] 19.3 测试房间管理前端功能
    - 编写房间过滤逻辑单元测试
    - 手动测试房间管理页面（创建测试清单并执行）
    - 可选：编写房间管理E2E测试（列表、详情、成员管理、目录发布）
    - _需求: 2.x_

- [ ] 20. 媒体管理前端功能完善
  - [ ] 20.1 完善媒体管理功能
    - 实现媒体隔离和保护
    - 实现远程媒体清理
    - _需求: 3.3, 3.4, 3.5_

  - [ ] 20.2 完善用户媒体统计功能
    - 实现用户媒体统计列表
    - 实现批量清理功能
    - _需求: 8.1, 8.2, 8.3_

  - [ ] 20.3 测试媒体管理前端功能
    - 编写媒体过滤逻辑单元测试
    - 手动测试媒体管理页面（创建测试清单并执行）
    - 可选：编写媒体管理E2E测试
    - _需求: 3.x, 8.x_

- [ ] 21. 检查点 - 阶段三完成
  - 房间管理所有功能实现完成
  - 媒体管理所有功能实现完成
  - 用户媒体统计功能就绪
  - 询问用户是否有问题

---

## 阶段四：新模块

- [ ] 22. 房间目录管理
  - [ ] 22.1 实现房间目录后端API
    - 创建RoomDirectoryAPI
    - 实现目录房间列表
    - 实现发布/取消发布功能
    - _需求: 4.1, 4.2, 4.3_

  - [ ] 22.2 测试房间目录API
    - 编写公开房间列表单元测试
    - 编写发布/取消发布功能单元测试
    - 编写批量操作单元测试
    - _需求: 4.1, 4.2, 4.3_

  - [ ] 22.3 实现房间目录前端页面
    - 创建房间目录列表页面
    - 实现批量发布/取消发布
    - 实现跳转到房间详情
    - _需求: 4.1, 4.2, 4.3, 4.4_

  - [ ] 22.4 测试房间目录前端功能
    - 手动测试房间目录页面
    - _需求: 4.x_

- [ ] 23. 注册令牌管理
  - [ ] 23.1 实现注册令牌后端API
    - 创建RegistrationTokensAPI
    - 实现令牌CRUD功能
    - 实现令牌生成和验证
    - _需求: 6.1, 6.2, 6.3, 6.4, 6.5_

  - [ ] 23.2 测试注册令牌API
    - 编写令牌CRUD功能单元测试
    - 编写令牌生成和验证单元测试
    - 编写令牌过滤功能单元测试
    - _需求: 6.1, 6.2, 6.3, 6.4, 6.5_

  - [ ] 23.3 实现注册令牌前端页面
    - 创建注册令牌列表页面
    - 实现令牌创建和编辑
    - 实现过滤功能
    - _需求: 6.1, 6.2, 6.3, 6.4, 6.5_

  - [ ] 23.4 测试注册令牌前端功能
    - 手动测试注册令牌页面
    - _需求: 6.x_

- [ ] 24. 举报管理
  - [ ] 24.1 实现举报管理后端API
    - 创建ReportsAPI
    - 实现举报列表和详情
    - 实现举报删除功能
    - _需求: 7.1, 7.2, 7.3_

  - [ ] 24.2 测试举报管理API
    - 编写举报列表和详情单元测试
    - 编写举报删除功能单元测试
    - 编写媒体预览功能单元测试
    - _需求: 7.1, 7.2, 7.3_

  - [ ] 24.3 实现举报管理前端页面
    - 创建举报列表页面
    - 实现举报详情查看（支持媒体预览）
    - 实现删除功能
    - _需求: 7.1, 7.2, 7.3_

  - [ ] 24.4 测试举报管理前端功能
    - 手动测试举报管理页面
    - _需求: 7.x_

- [ ] 25. 服务器操作
  - [ ] 25.1 实现服务器操作后端API
    - 创建ServerOperationsAPI
    - 实现危险操作确认
    - 实现服务器通知发送
    - 实现服务器重启功能
    - _需求: 18.1, 18.2, 18.3, 18.4_

  - [ ] 25.2 测试服务器操作API
    - 编写危险操作确认单元测试
    - 编写服务器通知发送单元测试
    - 编写服务器重启功能单元测试
    - _需求: 18.1, 18.2, 18.3, 18.4_

  - [ ] 25.3 实现服务器操作前端页面
    - 创建服务器操作列表
    - 实现操作确认对话框
    - 实现通知发送界面
    - 实现重启选项
    - _需求: 18.1, 18.2, 18.3, 18.4_

  - [ ] 25.4 测试服务器操作前端功能
    - 手动测试服务器操作页面
    - _需求: 18.x_

- [ ] 26. 自定义菜单和用户徽章
  - [ ] 26.1 实现自定义菜单功能
    - 创建自定义菜单配置API
    - 实现菜单项动态加载
    - _需求: 16.1_

  - [ ] 26.2 实现联系支持功能
    - 添加联系支持菜单项
    - 实现当前用户信息显示
    - _需求: 16.2, 16.3_

  - [ ] 26.3 实现用户徽章功能
    - 创建用户徽章配置
    - 实现徽章显示
    - _需求: 16.4_

  - [ ] 26.4 测试自定义菜单和徽章
    - 编写自定义菜单配置单元测试
    - 编写菜单项动态加载单元测试
    - 编写用户徽章配置单元测试
    - 手动测试自定义菜单和徽章功能
    - _需求: 16.x_

- [ ] 27. 检查点 - 阶段四完成
  - 所有新模块实现完成
  - 服务器操作功能就绪
  - 自定义菜单和徽章功能就绪
  - 询问用户是否有问题

---

## 综合测试和优化

- [ ] 28. 数据验证属性测试
  - [ ] 28.1 编写输入验证属性测试
    - **属性 5: 输入验证完整性**
    - 验证所有输入字段的验证规则
    - 验证边界值处理
    - 验证特殊字符处理
    - _需求: 10.1, 11.4, 11.5_

- [ ] 29. 端到端测试（可选）
  - [ ] 29.1 设置Playwright测试环境
    - 安装Playwright依赖
    - 配置测试环境
    - 创建测试辅助函数
    - _需求: 11.1, 11.2, 11.3_

  - [ ] 29.2 编写关键流程E2E测试
    - 编写认证流程E2E测试（登录、登出、令牌刷新）
    - 编写配置管理E2E测试（表单验证、提交、模板应用）
    - 编写用户管理E2E测试（创建、编辑、搜索、批量操作）
    - 编写房间管理E2E测试（列表、详情、成员管理）
    - 编写响应式布局E2E测试（桌面、移动、跨浏览器）
    - _需求: 10.x, 11.x, 12.x, 1.x, 2.x_

---

## 部署和优化

- [ ] 30. 构建优化
  - [ ] 30.1 优化构建配置
    - 配置WASM包大小优化
    - 实现静态资源压缩和缓存
    - 创建生产环境构建脚本
    - _需求: 性能需求_

  - [ ] 30.2 创建部署文档
    - 编写部署指南
    - 创建Docker容器化配置
    - 实现健康检查配置
    - _需求: 运维需求_

- [ ] 31. 最终检查点
  - 确保所有测试通过
  - 确保所有功能实现完成
  - 项目完成

---

## 注意事项

- 每个任务都引用了具体的需求以确保可追溯性
- 检查点确保增量验证和用户反馈
- 属性测试验证通用正确性属性
- 单元测试验证具体��例和边界情况
- 集成测试验证组件间交互和端到端流程
- 标记为可选的任务可以跳过以加快MVP开发

## 测试策略说明

### 测试分层架构

本项目采用分层测试策略，确保代码质量和功能正确性：

```
┌─────────────────────────────────────────┐
│   端到端测试 (E2E Tests)                │  ← 可选，使用Playwright
│   测试完整用户工作流程                   │
└─────────────────────────────────────────┘
           ↓
┌─────────────────────────────────────────┐
│   集成测试 (Integration Tests)          │  ← 必需，测试组件交互
│   测试API端点、数据库操作、认证流程      │
└─────────────────────────────────────────┘
           ↓
┌─────────────────────────────────────────┐
│   属性测试 (Property-Based Tests)       │  ← 必需，验证通用属性
│   使用随机数据验证系统不变性             │
└─────────────────────────────────────────┘
           ↓
┌─────────────────────────────────────────┐
│   单元测试 (Unit Tests)                 │  ← 必需，测试独立函数
│   测试独立函数、数据模型、业务逻辑       │
└─────────────────────────────────────────┘
```

### 后端测试策略

#### 1. 单元测试（必需）
- **目标**: 测试独立的函数和模块
- **工具**: Rust内置测试框架 + tokio-test
- **覆盖范围**:
  - 数据模型的序列化和反序列化
  - 业务逻辑函数
  - 错误处理逻辑
  - 数据验证规则
  - 工具函数

**示例**:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::user_admin_api::UserAdminAPI;
    use crate::utils::audit_logger::AuditLogger;

    /// 创建测试用的API实例
    fn create_test_api() -> UserAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        UserAdminAPI::new(audit_logger)
    }

    /// 测试用户验证功能 (需求1.5)
    #[tokio::test]
    async fn test_user_validation() {
        let api = create_test_api();
        let user = UserDetail {
            user_id: "@test:example.com".to_string(),
            username: "test".to_string(),
            display_name: Some("Test User".to_string()),
            avatar_url: None,
            is_admin: false,
            is_guest: false,
            is_deactivated: false,
            is_locked: false,
            is_suspended: false,
            is_erased: false,
            creation_ts: 0,
            threepids: vec![],
            external_ids: vec![],
            user_type: None,
        };
        
        let result = api.validate_user(&user).await;
        assert!(result.is_ok());
    }

    /// 测试无效用户ID格式 (需求1.5)
    #[tokio::test]
    async fn test_invalid_user_id() {
        let api = create_test_api();
        
        let result = api.validate_user_id("invalid").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Invalid user ID format");
    }

    /// 测试用户锁定功能 (需求1.5)
    #[tokio::test]
    async fn test_lock_user() {
        let api = create_test_api();
        let user_id = "@test:example.com".to_string();
        
        let result = api.lock_user(&user_id, "admin").await;
        assert!(result.is_ok());
        
        // 验证用户确实被锁定
        let user = api.get_user(&user_id).await.unwrap();
        assert!(user.is_locked);
    }
}
```

#### 2. 属性测试（必需）
- **目标**: 验证通用正确性属性
- **工具**: proptest
- **覆盖范围**:
  - 配置验证的一致性
  - 认证和授权的正确性
  - 列表操作的不变性
  - 批量操作的原子性
  - 数据验证的完整性

**示例**:
```rust
use proptest::prelude::*;

#[cfg(test)]
mod property_tests {
    use super::*;
    use crate::services::config_api::ConfigAPI;
    use crate::utils::audit_logger::AuditLogger;

    fn create_test_api() -> ConfigAPI {
        let audit_logger = AuditLogger::new(1000);
        ConfigAPI::new(audit_logger)
    }

    /// 属性测试：配置序列化/反序列化可逆性 (需求12.1)
    proptest! {
        #[test]
        fn test_config_serialization_roundtrip(
            server_name in "[a-z]{3,10}\\.[a-z]{2,5}",
            port in 1024u16..65535u16
        ) {
            let api = create_test_api();
            let config = ServerConfig {
                server_name,
                port,
                ..Default::default()
            };
            
            // 序列化后反序列化应该得到相同的配置
            let serialized = api.serialize_config(&config).unwrap();
            let deserialized = api.deserialize_config(&serialized).unwrap();
            prop_assert_eq!(config, deserialized);
        }
    }

    /// 属性测试：分页操作一致性 (需求1.2)
    proptest! {
        #[test]
        fn test_pagination_consistency(
            page in 1usize..100,
            page_size in 1usize..100
        ) {
            let api = create_test_api();
            
            // 分页操作应该返回一致的结果
            let result1 = api.list_users(page, page_size).await.unwrap();
            let result2 = api.list_users(page, page_size).await.unwrap();
            prop_assert_eq!(result1, result2);
        }
    }
}
```

#### 3. 集成测试（必需）
- **目标**: 测试组件间交互和端到端流程
- **工具**: Rust内置测试框架 + tokio + 测试数据库
- **覆盖范围**:
  - API端点的完整请求/响应流程
  - 数据库操作和事务
  - 认证和授权流程
  - 文件系统操作
  - 跨模块交互

**示例**:
```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::services::user_admin_api::UserAdminAPI;
    use crate::utils::audit_logger::AuditLogger;

    /// 创建测试用的API实例
    fn create_test_api() -> UserAdminAPI {
        let audit_logger = AuditLogger::new(1000);
        UserAdminAPI::new(audit_logger)
    }

    /// 集成测试：完整的用户生命周期 (需求1.x)
    #[tokio::test]
    async fn test_user_lifecycle() {
        let api = create_test_api();

        // 1. 创建用户
        let create_req = CreateUserRequest {
            username: "testuser".to_string(),
            password: Some("password123".to_string()),
            display_name: Some("Test User".to_string()),
            is_admin: false,
        };
        let user = api.create_user(create_req, "admin").await.unwrap();
        assert_eq!(user.username, "testuser");
        assert_eq!(user.display_name, Some("Test User".to_string()));

        // 2. 获取用户
        let fetched = api.get_user(&user.user_id, "admin").await.unwrap();
        assert_eq!(fetched.user_id, user.user_id);

        // 3. 更新用户
        let update_req = UpdateUserRequest {
            display_name: Some("Updated User".to_string()),
            is_admin: Some(false),
        };
        api.update_user(&user.user_id, update_req, "admin").await.unwrap();

        // 4. 锁定用户
        api.lock_user(&user.user_id, "admin").await.unwrap();
        let locked = api.get_user(&user.user_id, "admin").await.unwrap();
        assert!(locked.is_locked);

        // 5. 解锁用户
        api.unlock_user(&user.user_id, "admin").await.unwrap();
        let unlocked = api.get_user(&user.user_id, "admin").await.unwrap();
        assert!(!unlocked.is_locked);

        // 6. 停用用户
        api.deactivate_user(&user.user_id, false, true, "admin").await.unwrap();
        let deactivated = api.get_user(&user.user_id, "admin").await.unwrap();
        assert!(deactivated.is_deactivated);
    }

    /// 集成测试：批量用户操作 (需求1.18)
    #[tokio::test]
    async fn test_batch_user_operations() {
        let api = create_test_api();
        
        // 创建测试用户
        let user_ids = vec![
            "@user1:example.com".to_string(),
            "@user2:example.com".to_string(),
            "@user3:example.com".to_string(),
        ];
        
        for user_id in &user_ids {
            let create_req = CreateUserRequest {
                username: user_id.trim_start_matches('@').trim_end_matches(":example.com").to_string(),
                password: Some("password123".to_string()),
                ..Default::default()
            };
            api.create_user(create_req, "admin").await.unwrap();
        }
        
        // 批量锁定用户
        let lock_req = BatchUserOperationRequest {
            user_ids: user_ids.clone(),
            operation: BatchUserOperation::Lock,
        };
        let result = api.batch_operation(lock_req, "admin").await.unwrap();
        assert_eq!(result.success_count, 3);
        
        // 验证所有用户都被锁定
        for user_id in &user_ids {
            let user = api.get_user(user_id, "admin").await.unwrap();
            assert!(user.is_locked);
        }
    }
}
```

### 前端测试策略

由于Dioxus UI组件需要完整的运行时环境，前端测试采用不同的策略：

#### 1. 业务逻辑单元测试（部分可行）
- **目标**: 测试独立的业务逻辑函数
- **工具**: Rust内置测试框架
- **覆盖范围**:
  - 数据验证逻辑（不依赖DOM）
  - 数据转换逻辑
  - 搜索和过滤逻辑
  - 审计日志逻辑

**可以测试的示例**:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::validation::{validate_server_name, validate_ip_address};

    /// 测试服务器名称验证 (需求11.4)
    #[test]
    fn test_validate_server_name() {
        assert!(validate_server_name("example.com").is_ok());
        assert!(validate_server_name("sub.example.com").is_ok());
        assert!(validate_server_name("server.local").is_ok());
        assert!(validate_server_name("invalid").is_err());
        assert!(validate_server_name("").is_err());
        assert!(validate_server_name("-invalid.com").is_err());
    }

    /// 测试IP地址验证 (需求11.4)
    #[test]
    fn test_validate_ip_address() {
        assert!(validate_ip_address("127.0.0.1").is_ok());
        assert!(validate_ip_address("192.168.1.1").is_ok());
        assert!(validate_ip_address("::1").is_ok());
        assert!(validate_ip_address("invalid").is_err());
        assert!(validate_ip_address("").is_err());
    }

    /// 测试用户搜索过滤逻辑 (需求1.2)
    #[test]
    fn test_filter_users() {
        let users = vec![
            UserFilterResult {
                user_id: "@alice:example.com".to_string(),
                username: "alice".to_string(),
                display_name: Some("Alice Smith".to_string()),
            },
            UserFilterResult {
                user_id: "@bob:example.com".to_string(),
                username: "bob".to_string(),
                display_name: Some("Bob Jones".to_string()),
            },
            UserFilterResult {
                user_id: "@charlie:example.com".to_string(),
                username: "charlie".to_string(),
                display_name: Some("Charlie Brown".to_string()),
            },
        ];
        
        // 测试按用户名搜索
        let filtered = filter_users(&users, "al");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].username, "alice");
        
        // 测试按显示名搜索
        let filtered = filter_users(&users, "Bob");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].username, "bob");
        
        // 测试空搜索返回所有用户
        let filtered = filter_users(&users, "");
        assert_eq!(filtered.len(), 3);
    }
}
```

**无法测试的内容**:
- UI组件渲染（需要DOM）
- 用户交互（需要事件系统）
- 组件生命周期（需要Dioxus运行时）
- 状态管理（需要响应式系统）
- 路由导航（需要浏览器API）

#### 2. 手动测试（必需）
- **目标**: 验证UI组件和页面功能
- **工具**: 浏览器 + 开发者工具
- **流程**:
  1. 启动开发服务器: `dx serve`
  2. 访问 http://localhost:8080
  3. 按照测试清单逐项验证
  4. 记录测试结果

**测试清单示例**:
```markdown
### 配置表单页面测试清单
- [ ] 表单输入
  - [ ] 所有输入框正常显示
  - [ ] 必填标记（*）正确显示
  - [ ] 输入值正确更新
- [ ] 表单验证
  - [ ] 必填字段验证正确
  - [ ] 格式验证正确（IP、端口等）
  - [ ] 错误消息正确显示
- [ ] 表单提交
  - [ ] 提交按钮正常工作
  - [ ] Loading状态正确显示
  - [ ] 成功/失败消息正确显示
- [ ] 响应式布局
  - [ ] 桌面布局正常
  - [ ] 移动布局正常
  - [ ] 不同浏览器兼容
```

**详细指南**: 参考 `crates/admin-ui/TESTING_GUIDE.md`

#### 3. 端到端测试（可选）
- **目标**: 自动化测试完整用户工作流程
- **工具**: Playwright
- **时机**: UI功能稳定后实现
- **覆盖范围**:
  - 完整的用户工作流程
  - 跨页面导航
  - 表单提交和验证
  - 响应式布局
  - 跨浏览器兼容性

**示例**:
```javascript
// tests/e2e/config.spec.ts
import { test, expect } from '@playwright/test';

/**
 * E2E测试：配置管理完整流程 (需求12.x)
 */
test.describe('配置管理', () => {
  test.beforeEach(async ({ page }) => {
    // 1. 登录
    await page.goto('http://localhost:8080/login');
    await page.fill('input[name="username"]', 'admin');
    await page.fill('input[name="password"]', 'password');
    await page.click('button[type="submit"]');
    
    // 2. 导航到配置页面
    await page.click('a:has-text("配置管理")');
    await expect(page).toHaveURL(/.*\/config/);
  });

  test('配置表单验证和提交', async ({ page }) => {
    // 3. 修改配置
    await page.fill('input[name="server_name"]', 'test-server.com');
    await page.fill('input[name="port"]', '8008');
    
    // 4. 验证实时验证
    await page.fill('input[name="port"]', 'invalid');
    await expect(page.locator('.error-message')).toBeVisible();
    await expect(page.locator('.error-message')).toContainText('Invalid port');
    
    // 5. 提交表单
    await page.fill('input[name="port"]', '8008');
    await page.click('button:has-text("保存配置")');
    
    // 6. 验证loading状态
    const saveButton = page.locator('button:has-text("保存配置")');
    await expect(saveButton).toBeDisabled();
    await expect(saveButton.locator('.btn-spinner')).toBeVisible();
    
    // 7. 验证成功消息
    await expect(page.locator('.success-message')).toBeVisible();
    await expect(page.locator('.success-message')).toContainText('配置保存成功');
  });

  test('配置模板应用', async ({ page }) => {
    // 点击模板下拉框
    await page.click('select[name="config_template"]');
    
    // 选择开发模板
    await page.selectOption('select[name="config_template"]', 'development');
    
    // 验证配置已更新
    await expect(page.locator('input[name="server_name"]')).toHaveValue('dev-server.local');
    await expect(page.locator('input[name="port"]')).toHaveValue('8080');
  });
});

/**
 * E2E测试：用户管理完整流程 (需求1.x)
 */
test.describe('用户管理', () => {
  test.beforeEach(async ({ page }) => {
    // 登录
    await page.goto('http://localhost:8080/login');
    await page.fill('input[name="username"]', 'admin');
    await page.fill('input[name="password"]', 'password');
    await page.click('button[type="submit"]');
    
    // 导航到用户管理页面
    await page.click('a:has-text("用户管理")');
    await expect(page).toHaveURL(/.*\/users/);
  });

  test('用户创建和编辑', async ({ page }) => {
    // 点击新建用户按钮
    await page.click('button:has-text("新建用户")');
    
    // 填写用户信息
    await page.fill('input[name="username"]', 'newuser');
    await page.fill('input[name="display_name"]', 'New User');
    await page.click('button:has-text("生成密码")');
    
    // 提交表单
    await page.click('button:has-text("创建")');
    
    // 验证用户已创建
    await expect(page.locator('.success-message')).toBeVisible();
    await expect(page.locator('table.user-list')).toContainText('newuser');
  });

  test('用户搜索和过滤', async ({ page }) => {
    // 测试搜索功能
    await page.fill('input[name="search"]', 'admin');
    await page.click('button:has-text("搜索")');
    
    // 验证搜索结果
    await expect(page.locator('table.user-list tr')).toHaveCount(1);
    await expect(page.locator('table.user-list')).toContainText('admin');
  });

  test('批量用户操作', async ({ page }) => {
    // 选择用户
    await page.click('input[name="user_select_all"]');
    
    // 点击批量操作
    await page.click('button:has-text("批量操作")');
    await page.click('button:has-text("发送服务器通知")');
    
    // 填写通知内容
    await page.fill('textarea[name="notice_content"]', 'Test notice');
    await page.click('button:has-text("发送")');
    
    // 验证通知已发送
    await expect(page.locator('.success-message')).toContainText('通知已发送');
  });
});

/**
 * E2E测试：响应式布局 (需求11.1)
 */
test.describe('响应式布局', () => {
  test('桌面布局', async ({ page }) => {
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.goto('http://localhost:8080');
    
    // 侧边栏应该可见
    await expect(page.locator('.sidebar')).toBeVisible();
    await expect(page.locator('.main-content')).toHaveCSS('margin-left', '250px');
  });

  test('移动布局', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await page.goto('http://localhost:8080');
    
    // 侧边栏应该隐藏
    await expect(page.locator('.sidebar')).not.toBeVisible();
    
    // 点击菜单按钮显示侧边栏
    await page.click('.menu-toggle');
    await expect(page.locator('.sidebar')).toBeVisible();
  });
});
```

### 测试覆盖目标

| 组件类型 | 单元测试 | 属性测试 | 集成测试 | E2E测试 | 手动测试 |
|---------|---------|---------|---------|---------|---------|
| 后端API | ✅ 90%+ | ✅ 核心逻辑 | ✅ 所有端点 | - | - |
| 数据模型 | ✅ 100% | ✅ 序列化 | - | - | - |
| 业务逻辑 | ✅ 85%+ | ✅ 关键逻辑 | ✅ 跨模块 | - | - |
| 前端逻辑 | ⚠️ 可测试部分 | - | - | - | - |
| UI组件 | ❌ 不适用 | - | - | 🔄 可选 | ✅ 必需 |
| 前端页面 | ❌ 不适用 | - | - | 🔄 可选 | ✅ 必需 |

### 测试执行命令

```bash
# 运行所有后端测试
cargo test --package palpo-admin-ui --lib

# 运行特定模块测试
cargo test --package palpo-admin-ui api_client
cargo test --package palpo-admin-ui user_admin_api

# 运行属性测试
cargo test --package palpo-admin-ui --lib -- --ignored

# 运行集成测试
cargo test --package palpo-admin-ui --test '*'

# 启动前端开发服务器（手动测试）
cd crates/admin-ui
dx serve

# 运行E2E测试（未来）
npx playwright test
```

### 测试最佳实践

#### 后端测试
1. **使用测试数据库**: 每个测试使用独立的测试数据库
2. **清理测试数据**: 测试结束后清理所有测试数据
3. **Mock外部依赖**: 使用mock避免依赖外部服务
4. **测试边界情况**: 测试空输入、超长输入、特殊字符等
5. **测试错误处理**: 确保所有错误路径都被测试

#### 前端测试
1. **测试业务逻辑**: 优先测试不依赖DOM的业务逻辑
2. **使用测试清单**: 创建详细的手动测试清单
3. **记录测试结果**: 记录每次测试的结果和发现的问题
4. **测试多浏览器**: 在Chrome、Firefox、Safari中测试
5. **测试响应式**: 测试不同屏幕尺寸的布局

#### 属性测试
1. **选择合适的生成器**: 使用符合实际数据分布的生成器
2. **定义清晰的属性**: 属性应该简单、明确、可验证
3. **处理收缩**: 确保测试失败时能找到最小反例
4. **设置合理的测试次数**: 平衡测试时间和覆盖率

### 持续集成

测试应该集成到CI/CD流程中：

```yaml
# .github/workflows/test.yml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      
      - name: Setup Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      
      - name: Run unit tests
        run: cargo test --package palpo-admin-ui --lib
      
      - name: Run property tests
        run: cargo test --package palpo-admin-ui --lib -- --ignored
      
      - name: Run integration tests
        run: cargo test --package palpo-admin-ui --test '*'
      
      - name: Check code coverage
        run: cargo tarpaulin --package palpo-admin-ui --out Xml
      
      - name: Upload coverage
        uses: codecov/codecov-action@v2
```

### 参考文档

- `crates/admin-ui/TESTING_GUIDE.md` - 详细的手动测试指南
- `crates/admin-ui/FRONTEND_TESTING_STATUS.md` - 前端测试状态
- `crates/admin-ui/UI_TESTING_FINAL_SUMMARY.md` - UI测试总结
- `CONTRIBUTING.md` - 贡献指南和测试要求