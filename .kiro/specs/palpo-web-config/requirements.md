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

### 需求 8: 用户媒体统计

**用户故事:** 作为管理员，我希望查看用户媒体使用统计，以便了解存储使用情况。

#### 验收标准

1. WHEN 管理员访问用户媒体统计页面 THEN Admin_UI SHALL 显示所有用户的媒体使用统计，按媒体总大小降序排列
2. WHEN 管理员搜索用户 THEN Admin_UI SHALL 支持按用户ID或显示名搜索
3. WHEN 管理员执行批量操作 THEN Admin_UI SHALL 支持批量删除媒体和清理远程媒体

### 需求 9: 设备管理

**用户故事:** 作为管理员，我希望管理用户设备，以便控制用户会话。

#### 验收标准

1. WHEN 管理员访问设备管理页面 THEN Admin_UI SHALL 显示指定用户的所有设备列表
2. WHEN 管理员查看设备详情 THEN Admin_UI SHALL 显示设备ID、显示名、最后使用IP、最后活跃时间、用户代理
3. WHEN 管理员删除设备 THEN Admin_UI SHALL 支持删除用户设备

### 需求 10: 身份验证和授权

**用户故事:** 作为管理员，我希望系统安全地管理访问权限，以便保护服务器安全。

#### 验收标准

1. WHEN 用户访问管理界面 THEN Admin_UI SHALL 要求进行身份验证
2. WHEN 管理员登录 THEN Admin_UI SHALL 支持用户名密码和访问令牌两种登录方式
3. WHEN 管理员使用外部认证 THEN Admin_UI SHALL 支持外部认证提供商模式（如OIDC、LDAP）
4. WHEN 管理员会话过期 THEN Admin_UI SHALL 自动刷新访问令牌
5. WHEN 非授权用户访问 THEN Admin_UI SHALL 拒绝访问并记录尝试
6. WHEN 管理员配置CORS THEN Admin_UI SHALL 支持配置CORS凭证模式

### 需求 11: 用户界面和体验

**用户故事:** 作为管理员，我希望管理界面直观易用，以便高效地完成管理任务。

#### 验收标准

1. WHEN 管理员使用界面 THEN Admin_UI SHALL 提供响应式设计适配不同设备
2. WHEN 管理员浏览列表 THEN Admin_UI SHALL 支持自定义显示列、导出数据、分页
3. WHEN 管理员搜索资源 THEN Admin_UI SHALL 提供全局搜索和过滤功能
4. WHEN 管理员执行操作 THEN Admin_UI SHALL 显示操作确认对话框
5. WHEN 操作执行中 THEN Admin_UI SHALL 显示加载状态和进度指示
6. WHEN 操作完成 THEN Admin_UI SHALL 提供明确的成功或失败反馈
7. WHEN 管理员使用深色模式 THEN Admin_UI SHALL 支持深色和浅色主题切换
8. WHEN 管理员自定义界面 THEN Admin_UI SHALL 支持通过config.json配置文件自定义界面

### 需求 12: 配置管理

**用户故事:** 作为管理员，我希望通过web界面管理服务器配置，以便控制服务器的核心运行参数。

#### 验收标准

1. WHEN 管理员访问配置管理页面 THEN Admin_UI SHALL 显示服务器配置选项
2. WHEN 管理员编辑配置 THEN Admin_UI SHALL 提供配置项的编辑功能
3. WHEN 管理员保存配置 THEN Admin_UI SHALL 将配置写入配置文件
4. WHEN 管理员重载配置 THEN Admin_UI SHALL 支持重载配置而无需重启服务器
5. WHEN 管理员查看服务器版本 THEN Admin_UI SHALL 显示当前服务器版本信息

### 需求 13: 服务器状态和通知

**用户故事:** 作为管理员，我希望监控服务器状态和接收通知，以便及时了解服务器运行情况。

#### 验收标准

1. WHEN 管理员访问服务器状态页面 THEN Admin_UI SHALL 显示服务器健康状态和各项指标
2. WHEN 服务器状态异常 THEN Admin_UI SHALL 显示问题详情和帮助链接
3. WHEN 管理员接收服务器通知 THEN Admin_UI SHALL 显示通知指示器和通知列表
4. WHEN 管理员管理通知 THEN Admin_UI SHALL 支持查看和删除服务器通知

### 需求 14: 服务器命令

**用户故事:** 作为管理员，我希望执行服务器管理命令，以便进行维护操作。

#### 验收标准

1. WHEN 管理员访问命令页面 THEN Admin_UI SHALL 显示可用的管理命令列表
2. WHEN 管理员执行命令 THEN Admin_UI SHALL 支持执行单个命令或设置定时命令
3. WHEN 管理员设置定时命令 THEN Admin_UI SHALL 支持创建一次性或周期性定时命令
4. WHEN 管理员管理定时命令 THEN Admin_UI SHALL 支持查看、编辑和删除定时命令

### 需求 15: Appservice管理

**用户故事:** 作为管理员，我希望通过web界面管理Appservice，以便集成第三方服务和机器人。

#### 验收标准

1. WHEN 管理员访问Appservice管理 THEN Admin_UI SHALL 显示已注册的Appservice列表
2. WHEN 管理员注册新Appservice THEN Admin_UI SHALL 提供YAML配置上传和验证功能
3. WHEN 管理员查看Appservice配置 THEN Admin_UI SHALL 显示完整的YAML配置信息
4. WHEN 管理员注销Appservice THEN Admin_UI SHALL 确认删除并清理相关数据
5. WHEN Appservice配置无效 THEN Admin_UI SHALL 显示YAML解析错误和修正建议
6. WHEN 管理员管理AS用户 THEN Admin_UI SHALL 保护Appservice管理的用户不被意外修改

### 需求 16: 联系人支持和自定义菜单

**用户故事:** 作为管理员，我希望添加自定义菜单项和联系支持入口，以便提供更好的管理体验。

#### 验收标准

1. WHEN 管理员配置菜单 THEN Admin_UI SHALL 支持添加自定义菜单项
2. WHEN 管理员需要帮助 THEN Admin_UI SHALL 提供联系支持菜单项
3. WHEN 管理员查看用户信息 THEN Admin_UI SHALL 在顶部菜单显示当前用户信息
4. WHEN 管理员分配用户徽章 THEN Admin_UI SHALL 支持为用户分配特殊徽章

### 需求 17: 批量用户注册

**用户故事:** 作为管理员，我希望通过CSV文件批量注册用户，以便快速创建多个用户账户。

#### 验收标准

1. WHEN 管理员导入用户 THEN Admin_UI SHALL 支持通过CSV文件批量导入用户
2. WHEN 管理员验证CSV THEN Admin_UI SHALL 验证CSV格式和内容
3. WHEN CSV包含第三方标识 THEN Admin_UI SHALL 支持在导入时包含第三方标识信息
4. WHEN 导入完成 THEN Admin_UI SHALL 显示导入结果和任何错误

### 需求 18: 服务器操作

**用户故事:** 作为管理员，我希望执行服务器级操作，以便进行维护和故障处理。

#### 验收标准

1. WHEN 管理员访问服务器操作页面 THEN Admin_UI SHALL 显示可用的服务器操作列表
2. WHEN 管理员执行危险操作 THEN Admin_UI SHALL 要求确认并记录操作日志
3. WHEN 管理员发送服务器通知 THEN Admin_UI SHALL 支持向指定用户发送服务器通知
4. WHEN 管理员重启服务器 THEN Admin_UI SHALL 提供安全重启和强制重启选项