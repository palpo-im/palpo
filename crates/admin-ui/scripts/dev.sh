#!/bin/bash
# Development script for Palpo Admin UI

set -e

echo "🚀 Starting Palpo Admin UI development server..."
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
PORT="${PORT:-8080}"
OPEN_BROWSER="${OPEN_BROWSER:-true}"

while [[ $# -gt 0 ]]; do
    case $1 in
        --port)
            PORT="$2"
            shift 2
            ;;
        --no-open)
            OPEN_BROWSER="false"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--port PORT] [--no-open]"
            exit 1
            ;;
    esac
done

echo "🌐 Server will start on http://localhost:$PORT"
echo "🔥 Hot reload enabled"
echo "📝 Watching: src/, ../core/src/"
echo ""
echo "Press Ctrl+C to stop the server"
echo ""

# Start development server with hot reload
if [ "$OPEN_BROWSER" = "true" ]; then
    dx serve --hot-reload true --port "$PORT"
else
    dx serve --hot-reload true --port "$PORT" --no-open
fi