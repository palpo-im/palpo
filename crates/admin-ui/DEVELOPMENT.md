# Palpo Admin UI - 开发指南

## 快速开始

### 前置要求

- Rust 1.70+ (推荐使用 rustup)
- Dioxus CLI (自动安装)
- 现代浏览器 (Chrome, Firefox, Safari, Edge)

### 安装依赖

```bash
# 安装 Rust (如果尚未安装)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 添加 WASM 目标
rustup target add wasm32-unknown-unknown

# Dioxus CLI 会在首次运行脚本时自动安装
```

## 开发工作流

### 启动开发服务器

```bash
# 使用默认设置 (端口 8080，自动打开浏览器)
./scripts/dev.sh

# 自定义端口
PORT=3000 ./scripts/dev.sh

# 或使用命令行参数
./scripts/dev.sh --port 3000

# 不自动打开浏览器
./scripts/dev.sh --no-open
```

开发服务器特性:
- 🔥 热重载 - 代码更改自动刷新
- 📝 监听 `src/` 和 `../core/src/` 目录
- 🌐 默认地址: http://localhost:8080

### 构建生产版本

```bash
# 发布版本构建 (优化)
./scripts/build.sh

# 调试版本构建
./scripts/build.sh --debug

# 使用自定义配置文件
./scripts/build.sh --profile production
```

构建输出:
- 📁 输出目录: `dist/`
- 📦 包含优化的 WASM 文件
- 🗜️ 启用 wasm-opt 压缩 (level z)

### 运行测试

```bash
# 运行所有测试
./scripts/test.sh

# 仅运行单元测试
./scripts/test.sh --unit

# 仅运行集成测试
./scripts/test.sh --integration

# 监听模式 (自动重新运行)
./scripts/test.sh --watch
```

### 代码质量检查

```bash
# 运行所有检查 (check + clippy + fmt)
./scripts/check.sh

# 自动修复问题
./scripts/check.sh --fix
```

检查内容:
- ✅ Cargo check - 编译检查
- 📎 Clippy - 代码规范检查
- 🎨 Rustfmt - 代码格式检查

### 清理构建产物

```bash
./scripts/clean.sh
```

清理内容:
- 🗑️ Cargo build artifacts (`target/`)
- 🗑️ Dioxus dist directory (`dist/`)
- 🗑️ WASM target directory (`target-wasm/`)

## 项目结构

```
crates/admin-ui/
├── src/
│   ├── app.rs              # 主应用组件和路由
│   ├── lib.rs              # 库入口
│   ├── main.rs             # 程序入口
│   ├── components/         # 可复用UI组件
│   ├── hooks/              # 自定义Hooks
│   ├── middleware/         # 中间件
│   ├── models/             # 数据模型
│   ├── pages/              # 页面组件
│   ├── services/           # API服务层
│   └── utils/              # 工具函数
├── assets/                 # 静态资源
├── scripts/                # 开发脚本
├── examples/               # 示例代码
├── Cargo.toml              # Rust项目配置
├── Dioxus.toml             # Dioxus配置
└── tailwind.config.js      # Tailwind CSS配置
```

## 配置说明

### Dioxus.toml

关键配置项:

```toml
[web.watcher]
watch_path = ["src", "../core/src"]  # 监听路径
reload_html = true                    # HTML热重载
index_on_404 = true                   # SPA路由支持

[web.wasm-opt]
level = "z"                           # WASM优化级别
```

### Cargo.toml

条件编译配置:

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
# WASM特定依赖
gloo-net = "0.4"
gloo-storage = "0.3"

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
# Native特定依赖
tokio = { version = "1.0", features = ["fs", "macros", "rt"] }
```

## 开发技巧

### 热重载

代码更改会自动触发重新编译和浏览器刷新。监听的目录:
- `src/` - 前端代码
- `../core/src/` - 核心库代码

### 调试

在浏览器中使用开发者工具:

```rust
// 在代码中添加日志
web_sys::console::log_1(&"Debug message".into());
web_sys::console::error_1(&"Error message".into());
```

### 性能优化

发布构建自动启用:
- ✅ WASM优化 (wasm-opt level z)
- ✅ 代码压缩
- ✅ 死代码消除

### API代理

开发时可以配置API代理，在 `Dioxus.toml` 中:

```toml
[web.proxy]
backend = "http://localhost:8008"
```

## 常见问题

### Q: 如何安装 Dioxus CLI?

A: 脚本会自动安装。手动安装:
```bash
cargo install dioxus-cli
```

### Q: 热重载不工作?

A: 检查:
1. 文件是否在监听路径中
2. 浏览器控制台是否有错误
3. 尝试重启开发服务器

### Q: 构建失败?

A: 尝试:
```bash
./scripts/clean.sh
cargo update
./scripts/build.sh
```

### Q: WASM文件太大?

A: 确保使用发布构建:
```bash
./scripts/build.sh  # 默认是 --release
```

## 贡献指南

1. 运行代码检查: `./scripts/check.sh`
2. 运行测试: `./scripts/test.sh`
3. 确保所有检查通过
4. 提交代码

## 相关资源

- [Dioxus 文档](https://dioxuslabs.com/)
- [Rust WASM 指南](https://rustwasm.github.io/docs/book/)
- [TailwindCSS 文档](https://tailwindcss.com/docs)
