#!/bin/bash
# Check script for Palpo Admin UI - runs linting and formatting checks

set -e

echo "🔍 Running code quality checks..."
echo ""

# Parse command line arguments
FIX_MODE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --fix)
            FIX_MODE=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--fix]"
            exit 1
            ;;
    esac
done

# Run cargo check
echo "📋 Running cargo check..."
cargo check --all-targets
echo "✅ Cargo check passed"
echo ""

# Run cargo clippy
echo "📎 Running clippy..."
if [ "$FIX_MODE" = true ]; then
    cargo clippy --all-targets --fix --allow-dirty --allow-staged
else
    cargo clippy --all-targets -- -D warnings
fi
echo "✅ Clippy check passed"
echo ""

# Run cargo fmt
echo "🎨 Checking code formatting..."
if [ "$FIX_MODE" = true ]; then
    cargo fmt --all
    echo "✅ Code formatted"
else
    cargo fmt --all -- --check
    echo "✅ Code formatting check passed"
fi
echo ""

echo "✅ All checks passed!"
