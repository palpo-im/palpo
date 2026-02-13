#!/bin/bash
# Test script for Palpo Admin UI

set -e

echo "🧪 Running Palpo Admin UI tests..."
echo ""

# Parse command line arguments
TEST_TYPE="all"
WATCH_MODE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --unit)
            TEST_TYPE="unit"
            shift
            ;;
        --integration)
            TEST_TYPE="integration"
            shift
            ;;
        --watch)
            WATCH_MODE=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--unit|--integration] [--watch]"
            exit 1
            ;;
    esac
done

# Run tests based on type
case $TEST_TYPE in
    unit)
        echo "🔬 Running unit tests..."
        if [ "$WATCH_MODE" = true ]; then
            cargo watch -x "test --lib"
        else
            cargo test --lib
        fi
        ;;
    integration)
        echo "🔗 Running integration tests..."
        if [ "$WATCH_MODE" = true ]; then
            cargo watch -x "test --test '*'"
        else
            cargo test --test '*'
        fi
        ;;
    all)
        echo "🎯 Running all tests..."
        if [ "$WATCH_MODE" = true ]; then
            cargo watch -x test
        else
            cargo test
        fi
        ;;
esac

echo ""
echo "✅ Tests completed!"
