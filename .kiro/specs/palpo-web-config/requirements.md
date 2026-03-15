# 需求文档

## 介绍

Palpo Matrix服务器web管理界面是一个现代化的管理控制台，允许管理员通过web浏览器可视化地管理Palpo Matrix服务器的所有配置和运营功能。该系统参考synapse-admin等成熟项目，提供更直观、安全和用户友好的管理体验。

## 术语表

- **Palpo_Server**: Rust编写的Matrix服务器实现
- **Admin_UI**: 负责管理功能的web界面组件
- **Admin_User**: 具有管理权限的管理员用户
- **Resource**: 管理界面中的资源类型（如用户、房间、媒体等）
- **MXID**: Matrix用户ID或房间ID
- **MXC URL**: Matrix媒体内容的统一标识符
- **Registration_Token**: 用于控制用户注册的邀请码
- **Appservice**: 通过AS协议与Matrix服务器集成的第三方服务

## 需求

### 需求 1: 用户管理

**用户故事:** 作为管理员，我希望管理本地用户账户，以便控制用户访问和权限。

#### 验收标准

1. WHEN 管理员访问用户管理页面 THEN Admin_UI SHALL 显示所有用户列表，包括头像、用户名、显示名、创建时间、状态（是否锁定、停用、已删除）
2. WHEN 管理员创建新用户 THEN Admin_UI SHALL 提供用户名输入、显示名设置、密码生成或手动设置功能
3. WHEN 管理员输入用户名 THEN Admin_UI SHALL 实时检查用户名可用性并显示结果
4. WHEN 管理员设置密码 THEN Admin_UI SHALL 提供随机密码生成功能
5. WHEN 管理员编辑用户 THEN Admin_UI SHALL 支持以下功能：
   - 修改显示名和头像
   - 设置或取消管理员权限
   - 锁定/解锁用户账户
   - 停用/重新激活用户账户
   - 擦除用户数据
   - 暂停用户账户（MSC3823）
6. WHEN 管理员尝试删除自己 THEN Admin_UI SHALL 阻止操作并显示错误提示
7. WHEN 管理员管理用户设备 THEN Admin_UI SHALL 显示用户设备列表，包括设备ID、最后使用IP、最后活跃时间、用户代理
8. WHEN 管理员管理用户连接 THEN Admin_UI SHALL 显示用户当前连接信息，包括IP地址、最后活跃时间、用户代理
9. WHEN 管理员管理用户推送器 THEN Admin_UI SHALL 显示用户的推送器列表
10. WHEN 管理员管理用户媒体 THEN Admin_UI SHALL 显示用户上传的媒体文件列表
11. WHEN 管理员管理用户房间 THEN Admin_UI SHALL 显示用户加入的房间列表
12. WHEN 管理员管理用户成员资格 THEN Admin_UI SHALL 显示用户的所有房间成员资格记录
13. WHEN 管理员设置用户速率限制 THEN Admin_UI SHALL 允许配置用户的每秒消息数和突发次数
14. WHEN 管理员管理用户实验功能 THEN Admin_UI SHALL 允许启用或禁用用户的实验性功能
15. WHEN 管理员管理用户账户数据 THEN Admin_UI SHALL 允许查看和编辑用户的全局和房间级账户数据
16. WHEN 管理员管理用户第三方标识 THEN Admin_UI SHALL 支持添加和管理用户的邮箱、电话等第三方标识
17. WHEN 管理员管理用户SSO外部ID THEN Admin_UI SHALL 支持添加和管理用户的外部认证提供商关联
18. WHEN 管理员批量操作用户 THEN Admin_UI SHALL 支持批量发送服务器通知、删除用户

### 需求 2: 房间管理

**用户故事:** 作为管理员，我希望管理Matrix房间，以便维护社区秩序和处理问题房间。

#### 验收标准

1. WHEN 管理员访问房间管理页面 THEN Admin_UI SHALL 显示所有房间列表，包括房间ID、名称、成员数、加密状态、创建者
2. WHEN 管理员查看房间详情 THEN Admin_UI SHALL 显示房间完整信息，包括：
   - 基本信息：房间ID、名称、主题、规范别名、创建者
   - 详细信息：成员数、本地成员数、设备数、状态事件数、版本、加密方式
   - 权限设置：是否可联邦、是否公开、加入规则、访客访问、历史可见性
3. WHEN 管理员管理房间成员 THEN Admin_UI SHALL 显示房间成员列表，包括用户头像、ID、显示名、是否为访客、是否停用、是否锁定
4. WHEN 管理员管理房间状态 THEN Admin_UI SHALL 显示房间的状态事件列表，包括事件类型、时间戳、内容、发送者
5. WHEN 管理员管理房间媒体 THEN Admin_UI SHALL 显示房间上传的媒体文件列表
6. WHEN 管理员管理房间前沿终点 THEN Admin_UI SHALL 显示房间的前沿终点信息
7. WHEN 管理员发布房间到目录 THEN Admin_UI SHALL 支持将房间发布到房间目录
8. WHEN 管理员从目录取消发布 THEN Admin_UI SHALL 支持从房间目录取消发布房间
9. WHEN 管理员设置房间管理员 THEN Admin_UI SHALL 允许将用户提升为房间管理员
10. WHEN 管理员删除房间 THEN Admin_UI SHALL 提供房间删除功能，可选择是否封禁房间
11. WHEN 管理员过滤房间列表 THEN Admin_UI SHALL 支持按公开房间和空房间进行过滤

### 需求 3: 媒体管理

**用户故事:** 作为管理员，我希望管理媒体文件，以便控制存储使用和清理无用文件。

#### 验收标准

1. WHEN 管理员访问媒体管理页面 THEN Admin_UI SHALL 显示用户媒体使用统计，包括用户头像、用户ID、显示名、媒体数量、媒体总大小
2. WHEN 管理员删除媒体文件 THEN Admin_UI SHALL 支持通过媒体ID删除单个或多个媒体文件
3. WHEN 管理员隔离媒体文件 THEN Admin_UI SHALL 支持将媒体文件标记为隔离状态
4. WHEN 管理员保护媒体文件 THEN Admin_UI SHALL 支持保护媒体文件不被自动清理
5. WHEN 管理员清理远程媒体 THEN Admin_UI SHALL 支持按时间范围删除远程服务器上的媒体文件
6. WHEN 管理员访问用户媒体页面 THEN Admin_UI SHALL 显示指定用户上传的媒体文件列表
7. WHEN 管理员访问房间媒体页面 THEN Admin_UI SHALL 显示指定房间上传的媒体文件列表

### 需求 4: 房间目录管理

**用户故事:** 作为管理员，我希望管理房间目录，以便控制公共房间的可见性。

#### 验收标准

1. WHEN 管理员访问房间目录页面 THEN Admin_UI SHALL 显示所有公开房间列表，包括房间名称、房间ID、规范别名、主题、成员数
2. WHEN 管理员发布房间到目录 THEN Admin_UI SHALL 支持批量或单个发布房间到目录
3. WHEN 管理员从目录取消发布 THEN Admin_UI SHALL 支持批量或单个从目录取消发布房间
4. WHEN 管理员查看目录房间详情 THEN Admin_UI SHALL 支持跳转到房间详情页面

### 需求 5: 联邦目的地管理

**用户故事:** 作为管理员，我希望管理联邦连接目的地，以便监控和维护与其他服务器的连接。

#### 验收标准

1. WHEN 管理员访问联邦目的地页面 THEN Admin_UI SHALL 显示所有联邦目的地列表，包括目的地地址、失败时间、重试时间、重试间隔、最后成功流排序
2. WHEN 管理员搜索目的地 THEN Admin_UI SHALL 支持按目的地地址搜索
3. WHEN 管理员查看目的地详情 THEN Admin_UI SHALL 显示目的地详细信息和关联的房间列表
4. WHEN 管理员重连目的地 THEN Admin_UI SHALL 支持重置与指定目的地的连接
5. WHEN 联邦连接异常 THEN Admin_UI SHALL 在列表中以红色图标标识失败的连接

### 需求 6: 注册令牌管理

**用户故事:** 作为管理员，我希望管理注册令牌，以便控制新用户注册。

#### 验收标准

1. WHEN 管理员访问注册令牌页面 THEN Admin_UI SHALL 显示所有注册令牌列表，包括令牌、使用次数限制、待完成数、已完成数、过期时间
2. WHEN 管理员创建新令牌 THEN Admin_UI SHALL 支持生成随机令牌或指定令牌，设置使用次数限制和过期时间
3. WHEN 管理员编辑令牌 THEN Admin_UI SHALL 支持修改使用次数限制和过期时间
4. WHEN 管理员过滤令牌 THEN Admin_UI SHALL 支持按有效/无效状态过滤
5. WHEN 管理员删除令牌 THEN Admin_UI SHALL 支持删除注册令牌

### 需求 7: 举报管理

**用户故事:** 作为管理员，我希望管理用户举报，以便处理违规内容。

#### 验收标准

1. WHEN 管理员访问举报管理页面 THEN Admin_UI SHALL 显示所有举报列表，包括举报ID、接收时间、举报者、房间名称、分数
2. WHEN 管理员查看举报详情 THEN Admin_UI SHALL 显示举报完整信息，包括：
   - 基本信息：接收时间、举报者、房间、分数、原因
   - 事件详情：事件ID、发送者、时间戳、事件类型、事件内容（支持媒体预览）
3. WHEN 管理员删除举报 THEN Admin_UI SHALL 支持删除举报记录

### 需求 8: 设备管理

**用户故事:** 作为管理员，我希望管理用户设备，以便控制用户会话。

#### 验收标准

1. WHEN 管理员访问用户详情页面 THEN Admin_UI SHALL 显示用户的所有设备列表
2. WHEN 管理员查看设备详情 THEN Admin_UI SHALL 显示设备ID、显示名、最后使用IP、最后活跃时间、用户代理
3. WHEN 管理员删除设备 THEN Admin_UI SHALL 支持删除用户设备

### 需求 9: 身份验证和授权

**用户故事:** 作为管理员，我希望系统安全地管理访问权限，以便保护服务器安全。

#### 验收标准

1. WHEN 用户访问管理界面 THEN Admin_UI SHALL 要求进行身份验证
2. WHEN 管理员登录 THEN Admin_UI SHALL 支持用户名密码和访问令牌两种登录方式
3. WHEN 管理员使用外部认证 THEN Admin_UI SHALL 支持外部认证提供商模式（如OIDC、LDAP）
4. WHEN 管理员会话过期 THEN Admin_UI SHALL 自动刷新访问令牌
5. WHEN 非授权用户访问 THEN Admin_UI SHALL 拒绝访问并记录尝试
6. WHEN 管理员配置CORS THEN Admin_UI SHALL 支持配置CORS凭证模式

### 需求 10: 用户界面和体验

**用户故事:** 作为管理员，我希望管理界面直观易用，以便高效地完成管理任务。

#### 验收标准

1. WHEN 管理员使用界面 THEN Admin_UI SHALL 提供响应式设计适配不同设备
2. WHEN 管理员浏览列表 THEN Admin_UI SHALL 支持自定义显示列、导出数据、分页
3. WHEN 管理员搜索资源 THEN Admin_UI SHALL 提供全局搜索和过滤功能
4. WHEN 管理员执行操作 THEN Admin_UI SHALL 显示操作确认对话框
5. WHEN 操作执行中 THEN Admin_UI SHALL 显示加载状态和进度指示
6. WHEN 操作完成 THEN Admin_UI SHALL 提供明确的成功或失败反馈
7. WHEN 管理员使用深色模式 THEN Admin_UI SHALL 支持深色和浅色主题切换

### 需求 11: Palpo 服务器配置管理

**用户故事:** 作为管理员，我希望通过web界面管理 Palpo 服务器的配置，支持两种编辑方式：表单编辑（用户友好）和 TOML 文件编辑（高级用户），以便灵活控制服务器的核心运行参数。

#### 验收标准

**A. 表单编辑模式**

1. WHEN 管理员访问配置管理页面 THEN Admin_UI SHALL 默认显示表单编辑模式，包含以下配置分类：
   - 基础配置：服务器名称、绑定地址、监听端口、TLS 设置
   - 数据库配置：PostgreSQL 连接字符串、连接池大小、连接超时
   - 联邦配置：启用/禁用联邦、签名密钥路径、密钥验证
   - 认证配置：认证方式、会话超时、密码策略
   - 媒体配置：媒体存储路径、最大文件大小、清理策略
   - 网络配置：代理设置、速率限制、CORS 配置
   - 日志配置：日志级别、日志输出位置、日志轮转

2. WHEN 管理员编辑配置字段 THEN Admin_UI SHALL 提供以下功能：
   - 实时验证字段值（如端口范围、URL 格式、文件路径存在性）
   - 显示字段的描述和默认值
   - 支持撤销单个字段的修改
   - 标记已修改的字段（脏状态）

3. WHEN 管理员修改配置 THEN Admin_UI SHALL 启用"保存"和"重置"按钮，禁用状态下按钮不可点击

4. WHEN 管理员点击"验证配置"按钮 THEN Admin_UI SHALL 调用后端验证 API，显示验证结果：
   - 验证成功：显示"配置有效"提示
   - 验证失败：显示具体的错误信息和修复建议

5. WHEN 管理员保存配置 THEN Admin_UI SHALL：
   - 先验证所有字段
   - 验证通过后发送到后端保存
   - 显示"配置已保存"成功提示
   - 验证失败则显示错误信息，不保存

6. WHEN 管理员点击"重置"按钮 THEN Admin_UI SHALL 恢复到上次保存的配置

7. WHEN 管理员点击"重载配置"按钮 THEN Admin_UI SHALL 从服务器重新加载最新配置（不需要重启 Palpo）

8. WHEN 管理员查看配置页面 THEN Admin_UI SHALL 显示当前 Palpo 服务器版本信息

9. WHEN 管理员在配置页面搜索 THEN Admin_UI SHALL 支持模糊搜索配置项（按标签和描述）

**B. TOML 文件编辑模式**

10. WHEN 管理员点击"编辑 TOML 文件"标签页 THEN Admin_UI SHALL 显示 TOML 编辑器，包含：
    - 完整的 palpo.toml 文件内容
    - 语法高亮和代码格式化
    - 行号显示
    - 撤销/重做功能

11. WHEN 管理员在 TOML 编辑器中修改内容 THEN Admin_UI SHALL：
    - 实时显示修改状态（脏状态）
    - 启用"保存"和"重置"按钮
    - 支持 Ctrl+S 快捷键保存

12. WHEN 管理员点击"验证 TOML"按钮 THEN Admin_UI SHALL：
    - 验证 TOML 语法是否正确
    - 验证配置内容是否有效
    - 显示验证结果和错误位置（如有）

13. WHEN 管理员保存 TOML 文件 THEN Admin_UI SHALL：
    - 先验证 TOML 语法和内容
    - 验证通过后发送到后端保存
    - 显示"TOML 文件已保存"成功提示
    - 验证失败则显示错误信息和错误位置，不保存

14. WHEN 管理员在 TOML 编辑器中修改后切换到表单模式 THEN Admin_UI SHALL：
    - 提示"存在未保存的修改，是否保存？"
    - 用户可选择保存、放弃或继续编辑

15. WHEN 管理员在表单模式中修改后切换到 TOML 模式 THEN Admin_UI SHALL：
    - 提示"存在未保存的修改，是否保存？"
    - 用户可选择保存、放弃或继续编辑

**C. 启动前配置验证**

16. WHEN 管理员启动 Palpo 服务器 THEN Admin_UI SHALL：
    - 在启动前显示当前配置摘要（关键配置项）
    - 调用配置验证 API 检查配置是否有效
    - 如果配置无效，显示错误信息并阻止启动
    - 如果配置有效，显示"配置已验证"提示，允许启动
    - 启动成功后显示"服务器已启动"提示

**D. 配置导入/导出**

17. WHEN 管理员导入/导出配置 THEN Admin_UI SHALL 支持：
    - 导出当前配置为 JSON/YAML/TOML 格式
    - 导入配置文件（支持 JSON/YAML/TOML 格式）
    - 导入前验证配置格式和内容

## 需求 12: 用户管理增强

**用户故事:** 作为管理员，我希望系统完整支持用户管理的高级功能，以便与 Synapse Admin 保持功能对等。

#### 验收标准

1. WHEN 管理员擦除用户数据 THEN 系统 SHALL 支持 GDPR 合规的数据擦除，包括：
   - 用户 threepids（第三方身份邮箱/电话）
   - 用户外部身份关联（external_ids）
   - 用户账户数据
   - 擦除状态追踪（erased 字段）

2. WHEN 管理员尝试删除自己 THEN 系统 SHALL 阻止操作并显示错误提示"无法删除当前管理员账户"

3. WHEN 管理员管理用户设备 THEN 系统 SHALL 显示用户设备列表，包括：
   - 设备ID、显示名
   - 最后使用IP、最后活跃时间
   - 用户代理信息
   - 支持删除设备/强制登出

4. WHEN 管理员管理用户连接 THEN 系统 SHALL 显示用户当前连接信息，包括：
   - IP地址、连接时间
   - 用户代理
   - 连接状态

5. WHEN 管理员管理用户推送器 THEN 系统 SHALL 显示用户的推送器列表

6. WHEN 管理员管理用户房间 THEN 系统 SHALL 显示用户加入的房间列表

7. WHEN 管理员管理用户成员资格 THEN 系统 SHALL 显示用户的所有房间成员资格记录

8. WHEN 管理员设置用户速率限制 THEN 系统 SHALL 允许配置用户级速率限制：
   - 每秒消息数（messages_per_second）
   - 突发次数（burst_count）

9. WHEN 管理员管理用户实验功能 THEN 系统 SHALL 允许启用或禁用用户的实验性功能

10. WHEN 管理员管理用户账户数据 THEN 系统 SHALL 允许查看和编辑用户的：
    - 全局账户数据（global_account_data）
    - 房间级账户数据（room_account_data）

11. WHEN 管理员管理用户第三方标识 THEN 系统 SHALL 支持 threepids 管理：
    - 添加/删除邮箱（medium: email）
    - 添加/删除电话（medium: msisdn）
    - 追踪 added_at、validated_at 时间戳

12. WHEN 管理员管理用户SSO外部ID THEN 系统 SHALL 支持 external_ids 管理：
    - 关联外部身份提供商（auth_provider）
    - 外部用户ID（external_id）
    - 支持 OIDC、SAML 等 SSO 协议

13. WHEN 管理员批量操作用户 THEN 系统 SHALL 支持：
    - 批量发送服务器通知
    - 批量删除用户
    - CSV 批量导入用户（包含 threepids）

14. WHEN 系统返回用户信息 THEN 用户响应 SHALL 包含以下字段与 Synapse 对齐：
    - threepids: 第三方身份数组
    - external_ids: 外部身份数组
    - user_type: 用户类型（bot/support）
    - is_guest: 访客标识
    - appservice_id: 应用服务ID
    - erased: 数据擦除状态