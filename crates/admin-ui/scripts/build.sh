#!/bin/bash
# Build script for Palpo Admin UI

set -e

echo "🏗️  Building Palpo Admin UI..."
echo ""

# Check if Dioxus CLI is installed
if ! command -v dx &> /dev/null; then
    echo "⚠️  Dioxus CLI not found. Installing..."
    cargo install dioxus-cli
    echo "✅ Dioxus CLI installed successfully"
    echo ""
fi

# Display Dioxus version
echo "📦 Dioxus CLI version: $(dx --version)"
echo ""

# Parse command line arguments
BUILD_MODE="release"
PROFILE=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --debug)
            BUILD_MODE="debug"
            shift
            ;;
        --profile)
            PROFILE="$2"
            shift 2
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--debug] [--profile PROFILE]"
            exit 1
            ;;
    esac
done

# Build the project
if [ "$BUILD_MODE" = "release" ]; then
    echo "🔨 Building in release mode (optimized)..."
    if [ -n "$PROFILE" ]; then
        dx build --release --profile "$PROFILE"
    else
        dx build --release
    fi
else
    echo "🔨 Building in debug mode..."
    dx build
fi

echo ""
echo "✅ Build completed successfully!"
echo "📁 Output directory: dist/"
echo ""

# Display build artifacts
if [ -d "dist" ]; then
    echo "📦 Build artifacts:"
    ls -lh dist/ | tail -n +2
    echo ""
    
    # Calculate total size
    TOTAL_SIZE=$(du -sh dist/ | cut -f1)
    echo "💾 Total size: $TOTAL_SIZE"
fi