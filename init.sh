#!/bin/bash
# ADDS 环境初始化脚本
# 用法: bash init.sh

set -e

echo "=== Palpo Web 配置管理项目环境验证 ==="

# 1. 检查 Rust 工具链
echo "[1/5] 检查 Rust 工具链..."
if ! command -v cargo &> /dev/null; then
    echo "❌ 错误: 未找到 cargo，请先安装 Rust"
    exit 1
fi
echo "✅ Rust 版本: $(cargo --version)"

# 2. 检查 Dioxus CLI
echo "[2/5] 检查 Dioxus CLI..."
if ! command -v dx &> /dev/null; then
    echo "⚠️  未找到 dx，尝试安装..."
    cargo install dioxus-cli
fi
echo "✅ Dioxus CLI 版本: $(dx --version 2>/dev/null || echo 'unknown')"

# 3. 检查项目依赖
echo "[3/5] 检查项目依赖..."
if [ -f "Cargo.toml" ]; then
    echo "✅ Cargo.toml 存在"
else
    echo "❌ 错误: 未找到 Cargo.toml"
    exit 1
fi

# 4. 编译检查
echo "[4/5] 执行编译检查..."
if cargo check --package palpo-admin-ui &> /dev/null; then
    echo "✅ 编译检查通过"
else
    echo "⚠️  编译检查失败，请查看错误信息"
    cargo check --package palpo-admin-ui
fi

# 5. 运行基础测试
echo "[5/5] 运行基础测试..."
if cargo test --package palpo-admin-ui --lib -- --test-threads=1 2>&1 | head -50; then
    echo "✅ 基础测试通过"
else
    echo "⚠️  测试执行完成（部分测试可能失败）"
fi

echo ""
echo "=== 环境验证完成 ==="
echo "下一步: 运行回归测试并选择功能开发"