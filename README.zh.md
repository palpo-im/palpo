# Palpo：高性能 Matrix 主服务器

[![许可证](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](许可证)

[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)

Palpo 是一个用 Rust 编写的下一代 Matrix 主服务器实现，旨在实现高性能、可扩展性和强大的联邦功能。Palpo 基于 Salvo Web 框架和 PostgreSQL 数据库构建，在保持最低运维开销的同时，提供企业级实时消息传递和协作功能。

---

## 🌟 什么是 Matrix？

Matrix 是一个用于去中心化实时通信的开放标准。它支持：

- **端到端加密消息传递** - 默认安全对话

- **联邦架构** - 无单点控制或故障

- **互操作性** - 可与其他聊天平台（Slack、Discord、IRC 等）桥接

- **VoIP 和视频通话** - 内置语音和视频支持

- **丰富的消息传递功能** - 表情反应、主题讨论、文件共享等

访问 [matrix.org](https://matrix.org) 了解更多信息

---

## ✨ 为什么选择 Palpo？

### **性能至上**

- 使用 **Rust** 构建，确保内存安全和零成本抽象

- 由 **Salvo** Web 框架驱动，实现高吞吐量异步 I/O

- 使用 **PostgreSQL** 后端，确保可靠的数据持久化和 ACID 合规性

- 针对低延迟和高并发进行了优化

### **开发者友好**

- 简洁、模块化的代码库

- 完善的 API 文档

- 支持 Docker 部署

### **资源高效**

- 与参考实现相比，内存占用极低

- 高效的数据库查询模式

- 智能缓存策略

- 可横向扩展

---

## 🚀 快速入门

### 试用我们的演示服务器

**⚠️ 重要提示：测试服务器须知**

我们的测试服务器仅用于**评估和测试**：

- **URL**：`https://test.palpo.im`

- **⚠️ 此服务器上的所有数据将定期删除，恕不另行通知**注意事项**

- **请勿用于生产环境或存储重要对话**

- **出于测试目的，预计会频繁重置**

#### 连接 Cinny（网页客户端）

1. 打开 [Cinny](https://app.cinny.in/)

2. 点击主服务器选择中的“编辑”

3. 输入 `https://test.palpo.im` 作为您的自定义主服务器

4. 创建测试账号并开始聊天！

#### 连接 Element（桌面/移动端）

1. 下载 [Element](https://element.io/download)

2. 在登录界面，点击 homeserver 旁边的“编辑”按钮

3. 输入 `https://test.palpo.im`

4. 注册或登录

---

## 📦 安装

### 前提条件

- Rust 1.89 或更高版本

- PostgreSQL 16 或更高版本

- Linux、macOS 或 Windows（推荐使用 WSL2）

### 从源代码构建

```bash
# 克隆仓库

git clone https://github.com/palpo-im/palpo.git

cd palpo

# 构建项目

cargo build --release

# 复制示例配置

cp palpo-example.toml palpo.toml

# 编辑配置（设置您的域名、数据库凭据等）

nano palpo.toml

# 运行服务器

./target/release/palpo

```

### Docker 部署

```bash
# 拉取镜像

docker pull ghcr.io/palpo-im/palpo:latest

# 使用 docker-compose 运行

cd deploy/docker

docker-compose up -d

```

有关详细的部署说明，请参阅[安装](https://palpo.im/guide/installation/)。

---

## 🧪 当前进度

我们使用 [Complement](https://github.com/matrix-org/complement) 对 Matrix 规范进行全面的端到端测试。

- **测试结果**：[test_all.result.jsonl](tests/results/test_all.result.jsonl)

- **测试覆盖率**：持续改进对 Matrix 规范的遵循度

- **联合性**：积极与其他 homeserver 实现进行测试

---

## 🤝 贡献

我们欢迎各种形式的贡献！无论您是：

- 🐛 修复 bug

- ✨ 添加新功能

- 📝 改进文档

- 🧪 编写测试

- 🎨 改进用户体验

请阅读我们的[贡献指南](CONTRIBUTING.md)以开始参与。

---

---

## 🙏 致谢

Palpo 站在巨人的肩膀上。我们衷心感谢以下项目：

- **[Conduit](https://gitlab.com/famedly/conduit)** - 开创性的轻量级 Rust Matrix 主服务器

- **[Ruma](https://github.com/ruma/ruma)** - Matrix 的基本类型和协议实现

- **[Tuwunel](https://github.com/matrix-construct/tuwunel)** - 创新的 Matrix 服务器架构理念

- **[Salvo](https://github.com/salvo-rs/salvo)** - 高性能异步 Web 框架

- **[Matrix.org](https://matrix.org)** - 创建并维护 Matrix 协议

---

## 📄 许可

Palpo 采用 Apache License 2.0 许可。详情请参阅 [LICENSE](LICENSE)。

---

## 🔗 链接

- **网站**：[https://palpo.im](https://palpo.im)

- **源代码**：[https://github.com/palpo-im/palpo](https://github.com/palpo-im/palpo)

- **问题跟踪器**：[https://github.com/palpo-im/palpo/issues](https://github.com/palpo-im/palpo/issues)

- **演示服务器**：[https://test.palpo.im](https://test.palpo.im) ⚠️ 测试数据将被删除

---

## ⚠️ 重要通知

### 测试服务器数据保留

**测试服务器 `test.palpo.im` 仅用于评估。**

- 所有账号、房间和消息均可能随时被删除

- 数据不进行备份

- 服务可能会因更新而中断

- **请勿依赖此服务器进行任何重要通信**

### 生产环境使用

Palpo 正在积极开发中。为了确保稳定性，我们建议您：

- 始终备份您的数据库

- 在测试环境中测试更新

- 监控服务器日志和性能

---

**由 Palpo 社区倾情打造 ❤️**