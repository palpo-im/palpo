# Palpo 项目测试指导手册

本文档为开发者提供 Palpo 项目的测试指导，涵盖测试类型、运行方式和项目特定的测试要求。

## 目录

1. [测试类型](#测试类型)
2. [运行测试](#运行测试)
3. [编写测试](#编写测试)
4. [测试工具](#测试工具)
5. [CI/CD 流程](#cicd-流程)
6. [常见问题](#常见问题)
7. [参考资源](#参考资源)

---

## 测试类型

### 1. 单元测试

**位置**：各源文件中的 `tests.rs` 模块

**示例**：
- `crates/core/src/metadata/tests.rs` - 元数据测试
- `crates/core/src/signatures/tests.rs` - 签名验证测试
- `crates/core/src/state/tests.rs` - 状态解析测试
- `crates/core/src/state/event_auth/tests.rs` - 事件授权测试

**特点**：
- 测试单个函数或模块的功能
- 运行速度快（秒级）
- 无需外部依赖

### 2. Complement 测试

**简介**：Matrix 官方的规范兼容性测试套件，使用 Go 语言编写。

**位置**：`tests/complement/`

**包含内容**：
- 客户端-服务器 API 测试（CSAPI）
- Matrix 规范变更测试（MSC）
- Docker 构建配置和启动脚本

**用途**：验证 Palpo 实现符合 Matrix 规范。

**详情**：参见 [tests/README.md](tests/README.md) 和 [Complement 仓库](https://github.com/matrix-org/complement)。

### 3. SyTest 测试

**简介**：另一个 Matrix 规范测试框架，提供额外的兼容性验证。

**位置**：`tests/sytest/`

**包含内容**：
- `sytest-whitelist` - 已通过的测试列表
- `sytest-blacklist` - 未通过的测试列表
- `are-we-synapse-yet.py` - 测试结果分析工具

**用途**：补充验证 Matrix 规范兼容性。

---

## 运行测试

### 基本测试命令

```bash
# 运行所有测试
cargo test --all --all-features

# 运行特定 crate 的测试
cargo test -p palpo-core

# 运行特定测试
cargo test test_name

# 显示测试输出（包括 println! 等）
cargo test -- --nocapture

# 运行测试但不快速失败（显示所有失败）
cargo test --no-fail-fast
```

### 运行 Complement 测试

**前提条件**：
1. 安装 Docker
2. 克隆 Complement 仓库

```bash
# 克隆 Complement
git clone https://github.com/matrix-org/complement.git ../complement

# 运行测试
bash ./tests/complement.sh ../complement test.log test.jsonl
```

**测试流程**：
1. 构建基础 Docker 镜像（`complement-palpo-base`）
2. 构建测试 Docker 镜像（`complement-palpo-test`）
3. 运行 Go 测试套件
4. 生成测试日志（`test.log`）和结果文件（`test.jsonl`）

### 运行 SyTest 测试

**前提条件**：
1. 安装 Docker
2. 克隆 SyTest 和配置仓库

```bash
# 克隆仓库
git clone https://github.com/matrix-org/sytest.git ../sytest
git clone https://github.com/palpo-im/sytest-palpo.git ../sytest-palpo

# 运行测试
bash ./tests/run-sytest.sh
```

**测试输出**：
- 测试日志存储在 `tests/sytestout/logs/`
- 可使用 `tests/sytest/are-we-synapse-yet.py` 分析结果

### 提交前的完整检查

在提交代码前，运行完整的质量检查：

```bash
# 1. 代码格式检查
cargo fmt --all -- --check

# 2. 代码格式化（自动修复）
cargo +nightly fmt --all

# 3. Clippy 静态分析
cargo clippy --all --all-features -- -D warnings

# 4. 构建检查
cargo check --all --bins --examples --tests

# 5. 发布版本构建检查
cargo check --all --bins --examples --tests --release

# 6. 运行所有测试
cargo test --all --all-features --no-fail-fast

# 7. 拼写检查
typos
```

---

## 编写测试

### 测试要求

根据 [CONTRIBUTING.md](CONTRIBUTING.md) 的要求：

1. **为新功能添加测试**
   - 为新函数和模块添加单元测试
   - 为 API 端点添加集成测试

2. **测试覆盖范围**
   - 测试成功路径
   - 测试错误路径

3. **使用描述性测试名称**
   - 测试名称应清晰说明测试内容

### 测试示例

**基本单元测试结构**：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // 测试成功场景
        let result = function_to_test(valid_input);
        assert!(result.is_ok());

        // 测试错误场景
        let result = function_to_test(invalid_input);
        assert!(result.is_err());
    }
}
```

---

## 测试工具

### 测试辅助模块

项目在 `crates/core/src/test_utils.rs` 中提供了测试辅助函数：

**可用函数**：
- `alice()`, `bob()` - 创建测试用户
- `room_id()` - 创建测试房间 ID
- `event_id()` - 创建测试事件 ID
- `to_pdu_event()` - 创建 PDU 事件
- `TestStore` - 模拟事件存储
- `do_check()` - 状态解析验证
- `INITIAL_EVENTS()` - 初始化测试事件

**使用示例**：

```rust
use crate::test_utils::{alice, bob, room_id};

#[test]
fn test_room_membership() {
    let user = alice();      // 创建测试用户 Alice
    let room = room_id();    // 创建测试房间 ID
    let event = to_pdu_event(/* ... */);  // 创建 PDU 事件

    // 使用测试辅助函数
    let result = process_event(&event);
    assert!(result.is_ok());
}
```

### 快照测试

项目使用 `insta` 进行快照测试：

```rust
use insta::assert_snapshot;

#[test]
fn test_signature_output() {
    let signature = generate_signature();
    insta::assert_snapshot!(signature);
}
```

**快照测试工作流**：

```bash
# 运行测试
cargo test

# 如果快照不匹配，审查差异
cargo insta review

# 接受新快照（如果变更符合预期）
cargo insta accept

# 拒绝变更
cargo insta reject
```

---

## CI/CD 流程

### GitHub Actions 工作流

项目使用 `.github/workflows/quality-control.yml` 进行自动化测试。

**触发条件**：
- Pull Request（打开、同步、重新打开）
- 推送到 `main` 分支
- 仅当以下文件变更时触发：
  - `**.rs` - Rust 源文件
  - `**/Cargo.toml` - 依赖配置
  - `.github/workflows/**` - 工作流配置

**测试任务**：

1. **拼写检查**（typos）
2. **构建检查**
   - Debug 版本：`cargo check --all --bins --examples --tests`
   - Release 版本：`cargo check --all --bins --examples --tests --release`
3. **运行测试**
   - 命令：`cargo test --all --all-features --no-fail-fast -- --nocapture`
   - 超时时间：40 分钟

**测试环境**：
- 操作系统：Ubuntu Latest
- Rust 版本：Stable
- 架构：x86_64-unknown-linux-gnu

---

## 常见问题

### 1. 测试失败：找不到 PostgreSQL

**问题**：测试需要 PostgreSQL 但未安装或未运行。

**解决方案**：

```bash
# Ubuntu/Debian
sudo apt install postgresql postgresql-contrib
sudo systemctl start postgresql

# macOS
brew install postgresql
brew services start postgresql
```

### 2. Complement 测试失败：Docker 镜像问题

**问题**：Docker 镜像构建失败或不存在。

**解决方案**：

```bash
# 清理旧镜像
docker rmi complement-palpo-base complement-palpo-test

# 重新运行测试脚本（会自动重建）
bash ./tests/complement.sh ../complement test.log test.jsonl
```

### 3. 测试超时

**问题**：某些测试运行时间过长。

**解决方案**：

```bash
# 减少并发线程数
cargo test -- --test-threads=1 --nocapture

# 或运行特定测试
cargo test specific_test_name
```

### 4. 快照测试不匹配

**问题**：`insta` 快照测试失败。

**解决方案**：

```bash
# 审查快照差异
cargo insta review

# 如果变更是预期的，接受新快照
cargo insta accept
```

### 5. Clippy 警告

**问题**：代码不符合 Clippy 规范。

**解决方案**：

```bash
# 查看详细警告
cargo clippy --all --all-features

# 自动修复部分问题
cargo clippy --fix --all --all-features
```

**注意**：项目在 `Cargo.toml` 中配置了允许的警告（如 `result_large_err`、`type_complexity` 等），只有未配置的警告才会导致构建失败。

### 6. 拼写检查失败

**问题**：`typos` 工具检测到拼写错误。

**解决方案**：

```bash
# 查看拼写错误
typos

# 如果是专有名词，添加到 typos.toml 配置中
# 参见项目根目录的 typos.toml 文件
```

---

## 参考资源

- [CONTRIBUTING.md](CONTRIBUTING.md) - 项目贡献指南
- [tests/README.md](tests/README.md) - Complement 测试说明
- [Rust 测试文档](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Complement 测试框架](https://github.com/matrix-org/complement)
- [SyTest 测试框架](https://github.com/matrix-org/sytest)
- [Matrix 规范](https://spec.matrix.org/)

---

**最后更新**：2026 年
**维护者**：Palpo 开发团队
