#!/bin/bash
# Build script for Palpo Admin UI

set -e

echo "Building Palpo Admin UI..."

# Install Dioxus CLI if not present
if ! command -v dx &> /dev/null; then
    echo "Installing Dioxus CLI..."
    cargo install dioxus-cli
fi

# Build the project
dx build --release

echo "Build completed! Output in dist/"