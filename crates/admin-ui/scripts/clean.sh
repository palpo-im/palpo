#!/bin/bash
# Clean script for Palpo Admin UI

set -e

echo "🧹 Cleaning Palpo Admin UI build artifacts..."
echo ""

# Clean Cargo build artifacts
echo "🗑️  Removing Cargo build artifacts..."
cargo clean

# Clean Dioxus dist directory
if [ -d "dist" ]; then
    echo "🗑️  Removing dist/ directory..."
    rm -rf dist/
fi

# Clean target-wasm directory if it exists
if [ -d "target-wasm" ]; then
    echo "🗑️  Removing target-wasm/ directory..."
    rm -rf target-wasm/
fi

echo ""
echo "✅ Clean completed!"
